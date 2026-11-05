// SPDX-License-Identifier: AGPL-3.0-only
//
// Native model inference behind the `host-inference` import. ONNX Runtime is
// loaded from a shared library at runtime rather than linked, so one server binary
// serves every accelerator: a bundle's plain CPU library, or an operator's CUDA or
// OpenVINO build dropped in its place.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use ort::{
    environment::Environment,
    ep::{self, ExecutionProviderDispatch},
    session::Session,
    value::TensorRef,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Most tokens the static graphs an NPU needs can take; anything shorter is padded
/// to this, since the model's result doesn't change with padding.
const STATIC_TOKENS: usize = 256;

/// How long a model stays loaded with nothing to do. It holds over a gigabyte,
/// and a small server shouldn't pay that between a nightly scan and the next
/// search, which pays about a second to load it back.
const IDLE_UNLOAD: Duration = Duration::from_secs(15 * 60);

/// Where an operator wants models to run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InferenceDevice {
    /// Works everywhere, and fast enough that this is the default.
    #[default]
    Cpu,
    /// An NVIDIA, Windows or Apple GPU, sharing it with hardware transcodes.
    Gpu,
    /// An Intel, Qualcomm or Apple neural processor.
    Npu,
    /// The first of NPU, GPU and CPU that works here.
    Auto,
}

/// How to run models, fixed for the server's lifetime.
#[derive(Debug, Clone, Default)]
pub struct InferenceConfig {
    pub device: InferenceDevice,
    /// CPU threads per run; 0 uses half the physical cores, leaving the rest for
    /// whatever else the host is doing.
    pub threads: usize,
    /// The ONNX Runtime shared library. Unset looks in a release bundle's
    /// `onnxruntime/lib`, then wherever the system finds shared libraries.
    pub library: Option<PathBuf>,
}

/// Why a model couldn't run. Messages hold no more than the asset's own path.
#[derive(Debug, Clone, Error)]
pub enum InferenceError {
    #[error("ONNX Runtime is unavailable: {0}")]
    RuntimeUnavailable(String),
    #[error("could not load the model: {0}")]
    Load(String),
    #[error("inference failed: {0}")]
    Run(String),
}

struct Loaded {
    session: Arc<Mutex<Session>>,
    /// Sequences must be exactly this long, one at a time.
    static_tokens: Option<usize>,
    last_used: Instant,
}

/// Loaded models, shared by every plugin instance. One run at a time per model:
/// ONNX Runtime already spreads a run across its threads, and a search waiting on
/// an indexing batch only waits for that one batch.
#[derive(Default)]
pub struct Inference {
    config: InferenceConfig,
    models: Mutex<HashMap<PathBuf, Loaded>>,
    device_in_use: Mutex<Option<String>>,
    reaping: Mutex<bool>,
}

impl std::fmt::Debug for Inference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inference")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Inference {
    pub fn new(config: InferenceConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// The accelerator the last model actually loaded on, or None before one has.
    /// What was asked for isn't necessarily this: without the hardware or an ONNX
    /// Runtime built for it, models run on the CPU.
    pub fn device_in_use(&self) -> Option<String> {
        self.device_in_use
            .lock()
            .expect("inference mutex poisoned")
            .clone()
    }

    /// One vector per token sequence, in order, from the embedding model at
    /// `model`: an ONNX graph taking `input_ids` and `attention_mask`, with a
    /// `sentence_embedding` output. Loads the model on first use. Must be called
    /// within a tokio runtime; the work happens on a blocking thread.
    ///
    /// # Errors
    /// [`InferenceError`] when the runtime or model can't load or a run fails. A
    /// later call tries again.
    pub async fn embed_tokens(
        self: &Arc<Self>,
        model: &Path,
        tokens: Vec<Vec<u32>>,
    ) -> Result<Vec<Vec<f32>>, InferenceError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let this = Arc::clone(self);
        let model = model.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let (session, static_tokens) = this.session(&model)?;
            let mut session = session.lock().expect("inference mutex poisoned");
            match static_tokens {
                Some(length) => tokens
                    .iter()
                    .map(|sequence| {
                        let batch = Batch::new(std::slice::from_ref(sequence), Some(length));
                        Ok(run(&mut session, &batch)?.remove(0))
                    })
                    .collect(),
                None => run(&mut session, &Batch::new(&tokens, None)),
            }
        })
        .await
        .map_err(|e| InferenceError::Run(e.to_string()))?
        .inspect(|_| self.schedule_unload())
    }

    fn session(
        &self,
        model: &Path,
    ) -> Result<(Arc<Mutex<Session>>, Option<usize>), InferenceError> {
        let environment = RUNTIME
            .get_or_init(|| load_runtime(self.config.library.as_deref()))
            .as_ref()
            .map_err(Clone::clone)?;
        let mut models = self.models.lock().expect("inference mutex poisoned");
        if let Some(loaded) = models.get_mut(model) {
            loaded.last_used = Instant::now();
            return Ok((Arc::clone(&loaded.session), loaded.static_tokens));
        }
        let started = Instant::now();
        let (session, device, static_tokens) = load_session(environment, model, &self.config)?;
        tracing::info!(
            model = %model.display(),
            device,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "inference: model loaded"
        );
        *self.device_in_use.lock().expect("inference mutex poisoned") = Some(device.to_string());
        let session = Arc::new(Mutex::new(session));
        models.insert(
            model.to_path_buf(),
            Loaded {
                session: Arc::clone(&session),
                static_tokens,
                last_used: Instant::now(),
            },
        );
        Ok((session, static_tokens))
    }

    /// Starts the task that unloads idle models, if it isn't running. It ends when
    /// none are left, so a server that never embeds never has one.
    fn schedule_unload(self: &Arc<Self>) {
        let mut reaping = self.reaping.lock().expect("inference mutex poisoned");
        if std::mem::replace(&mut *reaping, true) {
            return;
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(IDLE_UNLOAD / 4).await;
                let mut models = this.models.lock().expect("inference mutex poisoned");
                models.retain(|_, loaded| loaded.last_used.elapsed() < IDLE_UNLOAD);
                if models.is_empty() {
                    tracing::debug!("inference: idle models unloaded");
                    *this.reaping.lock().expect("inference mutex poisoned") = false;
                    return;
                }
            }
        });
    }
}

/// ONNX Runtime allows one environment per process, so whichever configuration
/// asks first decides the library.
static RUNTIME: OnceLock<Result<Environment, InferenceError>> = OnceLock::new();

fn load_runtime(configured: Option<&Path>) -> Result<Environment, InferenceError> {
    let library = configured
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from))
        .or_else(|| {
            let bundled = noflex_core::bundle::dir("onnxruntime")?
                .join("lib")
                .join(library_name());
            bundled.is_file().then_some(bundled)
        })
        .unwrap_or_else(|| PathBuf::from(library_name()));
    let unavailable =
        |e: String| InferenceError::RuntimeUnavailable(format!("{}: {e}", library.display()));
    let environment = ort::init_from(&library)
        .map_err(|e| unavailable(e.to_string()))?
        .with_name("noflex")
        .build()
        .map_err(|e| unavailable(e.to_string()))?;
    tracing::info!(library = %library.display(), "inference: ONNX Runtime loaded");
    Ok(environment)
}

fn library_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// A provider to try, and what it demands of the model.
struct Candidate {
    name: &'static str,
    provider: ExecutionProviderDispatch,
    static_tokens: Option<usize>,
}

fn candidates(device: InferenceDevice) -> Vec<Candidate> {
    let npu = || {
        vec![
            Candidate {
                name: "openvino-npu",
                provider: ep::OpenVINO::default().with_device_type("NPU").build(),
                static_tokens: Some(STATIC_TOKENS),
            },
            Candidate {
                name: "qnn",
                provider: ep::QNN::default().build(),
                static_tokens: Some(STATIC_TOKENS),
            },
            Candidate {
                name: "coreml-neural-engine",
                provider: ep::CoreML::default()
                    .with_compute_units(ep::coreml::ComputeUnits::CPUAndNeuralEngine)
                    .build(),
                static_tokens: None,
            },
        ]
    };
    let gpu = || {
        vec![
            Candidate {
                name: "cuda",
                provider: ep::CUDA::default().build(),
                static_tokens: None,
            },
            Candidate {
                name: "directml",
                provider: ep::DirectML::default().build(),
                static_tokens: None,
            },
            Candidate {
                name: "coreml-gpu",
                provider: ep::CoreML::default()
                    .with_compute_units(ep::coreml::ComputeUnits::CPUAndGPU)
                    .build(),
                static_tokens: None,
            },
        ]
    };
    match device {
        InferenceDevice::Cpu => Vec::new(),
        InferenceDevice::Gpu => gpu(),
        InferenceDevice::Npu => npu(),
        InferenceDevice::Auto => npu().into_iter().chain(gpu()).collect(),
    }
}

/// ort falls back to the CPU quietly unless told otherwise, and then nobody knows
/// the accelerator they configured isn't in use. So each provider is tried on its
/// own, and the CPU comes last, on purpose, with a warning.
fn load_session(
    environment: &Environment,
    model: &Path,
    config: &InferenceConfig,
) -> Result<(Session, &'static str, Option<usize>), InferenceError> {
    for candidate in candidates(config.device) {
        let path = match candidate.static_tokens {
            Some(_) => static_graph(model),
            None => model.to_path_buf(),
        };
        match session_on(
            environment,
            &path,
            config,
            Some(candidate.provider.error_on_failure()),
        ) {
            Ok(session) => return Ok((session, candidate.name, candidate.static_tokens)),
            Err(e) => {
                tracing::info!(provider = candidate.name, error = %e, "inference: provider unavailable")
            }
        }
    }
    if config.device != InferenceDevice::Cpu {
        tracing::warn!(
            requested = ?config.device,
            "inference: no accelerator available; running on the CPU"
        );
    }
    let session = session_on(environment, model, config, None)?;
    Ok((session, "cpu", None))
}

/// The fixed-shape sibling NPUs need, `model_text.onnx` → `model_text_d256.onnx`,
/// if the model ships one.
fn static_graph(model: &Path) -> PathBuf {
    let sibling = model.file_stem().map(|stem| {
        let mut name = stem.to_os_string();
        name.push(format!("_d{STATIC_TOKENS}.onnx"));
        model.with_file_name(name)
    });
    sibling
        .filter(|path| path.is_file())
        .unwrap_or_else(|| model.to_path_buf())
}

fn session_on(
    environment: &Environment,
    model: &Path,
    config: &InferenceConfig,
    provider: Option<ExecutionProviderDispatch>,
) -> Result<Session, InferenceError> {
    let load = |e: ort::Error| InferenceError::Load(e.to_string());
    let threads = if config.threads == 0 {
        default_threads()
    } else {
        config.threads
    };
    let mut builder = Session::builder(environment)
        .map_err(load)?
        .with_intra_threads(threads)
        .map_err(|e| load(e.into()))?
        // Workers otherwise spin between runs, stealing cycles from everyone else
        // to save this rarely-busy pool a few microseconds.
        .with_config_entry("session.intra_op.allow_spinning", "0")
        .map_err(|e| load(e.into()))?
        // Without this, the weights of a quantized graph are dequantized again on
        // every run: measured on EmbeddingGemma 2's int8 export, a query takes 227ms
        // with it unset and 29ms set, and memory falls by 800MB.
        .with_config_entry("session.disable_quant_qdq", "1")
        .map_err(|e| load(e.into()))?;
    if let Some(provider) = provider {
        builder = builder
            .with_execution_providers([provider])
            .map_err(|e| load(e.into()))?;
    }
    builder.commit_from_file(model).map_err(load)
}

fn default_threads() -> usize {
    sysinfo::System::physical_core_count()
        .map_or(1, |cores| cores / 2)
        .max(1)
}

/// Token sequences padded into the rectangle a graph takes.
#[derive(Debug, PartialEq, Eq)]
struct Batch {
    rows: usize,
    columns: usize,
    ids: Vec<i64>,
    mask: Vec<i64>,
}

impl Batch {
    /// Pads to `length`, cutting anything longer, or else to the longest sequence.
    fn new(tokens: &[Vec<u32>], length: Option<usize>) -> Self {
        let columns = length
            .or_else(|| tokens.iter().map(Vec::len).max())
            .unwrap_or(0);
        let mut ids = vec![0; tokens.len() * columns];
        let mut mask = vec![0; tokens.len() * columns];
        for (row, sequence) in tokens.iter().enumerate() {
            for (column, token) in sequence.iter().take(columns).enumerate() {
                ids[row * columns + column] = i64::from(*token);
                mask[row * columns + column] = 1;
            }
        }
        Self {
            rows: tokens.len(),
            columns,
            ids,
            mask,
        }
    }
}

fn run(session: &mut Session, batch: &Batch) -> Result<Vec<Vec<f32>>, InferenceError> {
    let failed = |e: ort::Error| InferenceError::Run(e.to_string());
    let shape = [batch.rows, batch.columns];
    let inputs = ort::inputs![
        "input_ids" => TensorRef::from_array_view((shape, batch.ids.as_slice())).map_err(failed)?,
        "attention_mask" => TensorRef::from_array_view((shape, batch.mask.as_slice())).map_err(failed)?,
    ];
    let outputs = session.run(inputs).map_err(failed)?;
    let output = outputs.get("sentence_embedding").ok_or_else(|| {
        InferenceError::Run("the model has no `sentence_embedding` output".to_string())
    })?;
    let (_, values) = output.try_extract_tensor::<f32>().map_err(failed)?;
    if batch.rows == 0 || values.len() % batch.rows != 0 {
        return Err(InferenceError::Run(format!(
            "{} values don't divide among {} sequences",
            values.len(),
            batch.rows
        )));
    }
    Ok(values
        .chunks_exact(values.len() / batch.rows)
        .map(<[f32]>::to_vec)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_pad_to_the_longest_and_mask_the_padding() {
        let batch = Batch::new(&[vec![5, 6, 7], vec![8]], None);
        assert_eq!((batch.rows, batch.columns), (2, 3));
        assert_eq!(batch.ids, [5, 6, 7, 8, 0, 0]);
        assert_eq!(batch.mask, [1, 1, 1, 1, 0, 0]);
    }

    #[test]
    fn static_batches_pad_or_cut_to_the_fixed_length() {
        let batch = Batch::new(&[vec![5, 6, 7]], Some(2));
        assert_eq!((batch.ids, batch.mask), (vec![5, 6], vec![1, 1]));
        let batch = Batch::new(&[vec![5]], Some(3));
        assert_eq!((batch.ids, batch.mask), (vec![5, 0, 0], vec![1, 0, 0]));
    }

    #[test]
    fn static_graph_is_used_only_when_the_model_ships_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let model = dir.path().join("model_text.onnx");
        assert_eq!(static_graph(&model), model);
        let sibling = dir.path().join("model_text_d256.onnx");
        std::fs::write(&sibling, b"").expect("sibling");
        assert_eq!(static_graph(&model), sibling);
    }
}
