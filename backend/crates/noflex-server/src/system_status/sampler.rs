// SPDX-License-Identifier: AGPL-3.0-only
//
// Host and process sampling. sysinfo reports CPU usage as the delta
// between two refreshes and network counters as running totals, so the
// sampler is stateful: it keeps the previous totals and derives
// per-second rates from its own clock rather than trusting the
// refresh cadence.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

use noflex_graphql::admin_status::{
    CpuCore, CpuStatus, HostInfo, LoadAverage, MemoryStatus, NetworkInterface, NetworkTotals,
    ProcessStatus,
};
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System,
};
use time::OffsetDateTime;

/// One host reading, minus the parts the service assembles itself.
#[derive(Debug, Clone, Default)]
pub struct HostSample {
    pub host: HostInfo,
    pub cpu: CpuStatus,
    pub memory: MemoryStatus,
    pub process: ProcessStatus,
    pub network: Vec<NetworkInterface>,
    pub network_total: NetworkTotals,
}

pub struct HostSampler {
    system: Option<System>,
    networks: Option<Networks>,
    pid: Option<Pid>,
    prev_net: HashMap<String, (u64, u64)>,
    prev_at: Option<Instant>,
}

impl HostSampler {
    /// Never fails: a platform sysinfo can't read yields zeroed figures.
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_usage().with_frequency())
                .with_memory(MemoryRefreshKind::everything()),
        )
        .ok();
        Self {
            system,
            networks: Networks::new_with_refreshed_list().ok(),
            pid: sysinfo::get_current_pid().ok(),
            prev_net: HashMap::new(),
            prev_at: None,
        }
    }

    /// Refresh and read. Blocking (reads `/proc`); call off the async
    /// executor. CPU figures are meaningful from the second call on.
    pub fn sample(&mut self) -> HostSample {
        let now = Instant::now();
        let elapsed = self.prev_at.map(|t| now.duration_since(t).as_secs_f64());
        self.prev_at = Some(now);

        let mut out = HostSample {
            host: host_info(),
            ..HostSample::default()
        };
        if let Some(sys) = self.system.as_mut() {
            sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage().with_frequency());
            sys.refresh_memory();
            if let Some(pid) = self.pid {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[pid]),
                    true,
                    ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory()
                        .with_tasks(),
                );
            }
            out.cpu = cpu_status(sys);
            out.memory = memory_status(sys);
            out.process = self
                .pid
                .and_then(|pid| sys.process(pid))
                .map(|p| process_status(p, out.cpu.logical_cores))
                .unwrap_or_default();
        }
        if let Some(networks) = self.networks.as_mut() {
            networks.refresh(true);
            let (ifaces, totals) = network_status(networks, &mut self.prev_net, elapsed);
            out.network = ifaces;
            out.network_total = totals;
        }
        out
    }
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn host_info() -> HostInfo {
    let booted_at = System::boot_time()
        .ok()
        .and_then(|s| OffsetDateTime::from_unix_timestamp(i64::try_from(s).ok()?).ok());
    HostInfo {
        hostname: System::host_name().ok(),
        os_name: System::name().ok(),
        os_version: System::long_os_version()
            .ok()
            .or_else(|| System::os_version().ok()),
        kernel_version: System::kernel_version().ok(),
        arch: System::cpu_arch(),
        booted_at,
        uptime_seconds: System::uptime().ok(),
    }
}

fn cpu_status(sys: &System) -> CpuStatus {
    let cpus = sys.cpus();
    CpuStatus {
        usage_percent: sys.global_cpu_usage(),
        brand: cpus
            .first()
            .map(|c| c.brand().trim().to_owned())
            .unwrap_or_default(),
        physical_cores: System::physical_core_count()
            .ok()
            .and_then(|n| u32::try_from(n).ok()),
        logical_cores: u32::try_from(cpus.len()).unwrap_or(u32::MAX),
        cores: cpus
            .iter()
            .enumerate()
            .map(|(i, c)| CpuCore {
                index: u32::try_from(i).unwrap_or(u32::MAX),
                usage_percent: c.usage(),
                frequency_mhz: c.frequency(),
            })
            .collect(),
        load_average: System::load_average().ok().map(|l| LoadAverage {
            one: l.one,
            five: l.five,
            fifteen: l.fifteen,
        }),
    }
}

fn memory_status(sys: &System) -> MemoryStatus {
    MemoryStatus {
        total_bytes: sys.total_memory(),
        used_bytes: sys.used_memory(),
        available_bytes: sys.available_memory(),
        swap_total_bytes: sys.total_swap(),
        swap_used_bytes: sys.used_swap(),
        cgroup_limit_bytes: sys
            .cgroup_limits()
            .map(|c| c.total_memory)
            // An unlimited cgroup reports the host total or more.
            .filter(|&limit| limit > 0 && limit < sys.total_memory()),
    }
}

fn process_status(p: &sysinfo::Process, logical_cores: u32) -> ProcessStatus {
    let cpu = p.cpu_usage();
    ProcessStatus {
        pid: p.pid().as_u32(),
        cpu_percent: cpu,
        cpu_percent_of_host: if logical_cores == 0 {
            cpu
        } else {
            cpu / logical_cores as f32
        },
        rss_bytes: p.memory(),
        virtual_bytes: p.virtual_memory(),
        threads: p.tasks().and_then(|t| u32::try_from(t.len()).ok()),
        open_files: p.open_files().and_then(|n| u32::try_from(n).ok()),
        open_files_limit: p.open_files_limit().and_then(|n| u32::try_from(n).ok()),
    }
}

fn network_status(
    networks: &Networks,
    prev: &mut HashMap<String, (u64, u64)>,
    elapsed_secs: Option<f64>,
) -> (Vec<NetworkInterface>, NetworkTotals) {
    let mut next = HashMap::with_capacity(networks.len());
    let mut totals = NetworkTotals::default();
    let mut ifaces: Vec<NetworkInterface> = networks
        .iter()
        .map(|(name, data)| {
            let name = name.to_string_lossy().into_owned();
            let rx = data.total_received();
            let tx = data.total_transmitted();
            let (rx_rate, tx_rate) = match (prev.get(&name), elapsed_secs) {
                (Some(&(prx, ptx)), Some(dt)) => (rate(prx, rx, dt), rate(ptx, tx, dt)),
                _ => (0.0, 0.0),
            };
            next.insert(name.clone(), (rx, tx));
            let addresses: Vec<String> = data
                .ip_networks()
                .iter()
                .map(|n| format!("{}/{}", n.addr, n.prefix))
                .collect();
            let loopback = name == "lo"
                || (!data.ip_networks().is_empty()
                    && data.ip_networks().iter().all(|n| n.addr.is_loopback()));
            if !loopback {
                totals.rx_bytes_per_sec += rx_rate;
                totals.tx_bytes_per_sec += tx_rate;
            }
            let mac = data.mac_address();
            NetworkInterface {
                name,
                loopback,
                state: data.operational_state().to_string().to_lowercase(),
                mac_address: (!mac.is_unspecified()).then(|| mac.to_string()),
                addresses,
                rx_bytes_per_sec: rx_rate,
                tx_bytes_per_sec: tx_rate,
                rx_total_bytes: rx,
                tx_total_bytes: tx,
                rx_errors_total: data.total_errors_on_received(),
                tx_errors_total: data.total_errors_on_transmitted(),
            }
        })
        .collect();
    ifaces.sort_by(|a, b| a.name.cmp(&b.name));
    *prev = next;
    (ifaces, totals)
}

/// Bytes per second between two readings of a monotonic counter. A
/// counter that went backwards (interface reset, 32-bit wrap) or a
/// non-positive interval yields 0 rather than a bogus spike.
pub fn rate(prev: u64, cur: u64, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 || cur < prev {
        return 0.0;
    }
    (cur - prev) as f64 / elapsed_secs
}

/// Capacity of the filesystem holding a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsUsage {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

/// Filesystem capacity for `path`. Blocking, and can hang on a dead
/// network mount: call off the executor with a timeout.
#[cfg(unix)]
pub fn fs_usage(path: &Path) -> std::io::Result<FsUsage> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};

    let c_path = CString::new(path.as_os_str().as_bytes())?;
    let mut st = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is NUL-terminated and outlives the call; `st` is
    // a valid out-pointer statvfs fully initialises on success.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), st.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: rc == 0, so statvfs wrote the struct.
    let st = unsafe { st.assume_init() };
    // statvfs field widths differ per platform (u32 on some BSDs).
    #[allow(clippy::unnecessary_cast)]
    let (frsize, blocks, bavail, bfree) = (
        if st.f_frsize == 0 {
            st.f_bsize as u64
        } else {
            st.f_frsize as u64
        },
        st.f_blocks as u64,
        st.f_bavail as u64,
        st.f_bfree as u64,
    );
    let total = blocks.saturating_mul(frsize);
    Ok(FsUsage {
        total,
        available: bavail.saturating_mul(frsize),
        used: total.saturating_sub(bfree.saturating_mul(frsize)),
    })
}

#[cfg(not(unix))]
pub fn fs_usage(_path: &Path) -> std::io::Result<FsUsage> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "filesystem usage is only sampled on unix",
    ))
}

/// One `/proc/self/mounts` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub mount_point: PathBuf,
    pub fs_type: String,
}

/// The current mount table, empty where `/proc` isn't available.
pub fn read_mounts() -> Vec<Mount> {
    std::fs::read_to_string("/proc/self/mounts")
        .map(|s| parse_mounts(&s))
        .unwrap_or_default()
}

pub fn parse_mounts(table: &str) -> Vec<Mount> {
    table
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _device = parts.next()?;
            let mount_point = unescape_mount(parts.next()?);
            let fs_type = parts.next()?.to_owned();
            Some(Mount {
                mount_point: PathBuf::from(mount_point),
                fs_type,
            })
        })
        .collect()
}

/// `/proc/self/mounts` escapes space, tab, newline and backslash as
/// three-digit octal (`\040`).
fn unescape_mount(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 3 < bytes.len())
            .then(|| &bytes[i + 1..i + 4])
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)))
            .and_then(|d| {
                let v = d.iter().fold(0u16, |acc, b| acc * 8 + u16::from(b - b'0'));
                u8::try_from(v).ok()
            });
        if let Some(v) = octal {
            out.push(v);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mount holding `path`: the longest mount point that is a
/// component-wise prefix. Later rows shadow earlier ones on ties, as
/// the kernel does for stacked mounts.
pub fn mount_for<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .fold(None, |best: Option<&Mount>, m| match best {
            Some(b) if b.mount_point.as_os_str().len() > m.mount_point.as_os_str().len() => Some(b),
            _ => Some(m),
        })
}

/// Total bytes of regular files under `root`, without following
/// symlinks. Stops after `max_entries` entries and reports whether it
/// finished. Blocking.
pub fn dir_size(root: &Path, max_entries: usize) -> (u64, bool) {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries {
                return (total, false);
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }
    (total, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_divides_delta_by_interval() {
        assert_eq!(rate(1_000, 3_000, 2.0), 1_000.0);
        assert_eq!(rate(0, 0, 2.0), 0.0);
    }

    #[test]
    fn rate_ignores_resets_and_bad_intervals() {
        assert_eq!(rate(5_000, 100, 2.0), 0.0);
        assert_eq!(rate(0, 100, 0.0), 0.0);
        assert_eq!(rate(0, 100, -1.0), 0.0);
    }

    #[test]
    fn mount_table_parses_and_unescapes() {
        let table = "/dev/nvme0n1p2 / ext4 rw 0 0\n\
                     nas:/media /mnt/my\\040media nfs4 rw 0 0\n\
                     tmpfs /tmp tmpfs rw 0 0\n";
        let mounts = parse_mounts(table);
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[1].mount_point, PathBuf::from("/mnt/my media"));
        assert_eq!(mounts[1].fs_type, "nfs4");
    }

    #[test]
    fn mount_for_picks_longest_component_prefix() {
        let mounts =
            parse_mounts("a / ext4 rw 0 0\nb /mnt/media nfs4 rw 0 0\nc /mnt/med xfs rw 0 0\n");
        let m = mount_for(&mounts, Path::new("/mnt/media/movies")).map(|m| m.fs_type.as_str());
        assert_eq!(m, Some("nfs4"));
        let m = mount_for(&mounts, Path::new("/mnt/medx")).map(|m| m.fs_type.as_str());
        assert_eq!(m, Some("ext4"));
    }

    #[test]
    fn dir_size_sums_files_and_respects_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a"), [0u8; 10]).expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        std::fs::write(dir.path().join("sub/b"), [0u8; 5]).expect("write");
        assert_eq!(dir_size(dir.path(), 100), (15, true));
        assert!(!dir_size(dir.path(), 1).1);
    }

    #[cfg(unix)]
    #[test]
    fn fs_usage_reports_a_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let usage = fs_usage(dir.path()).expect("statvfs");
        assert!(usage.total > 0);
        assert!(usage.available <= usage.total);
        assert!(fs_usage(Path::new("/definitely/not/here")).is_err());
    }

    #[test]
    fn sampler_produces_rates_on_second_sample() {
        let mut s = HostSampler::new();
        let first = s.sample();
        assert!(first.network.iter().all(|n| n.rx_bytes_per_sec == 0.0));
        std::thread::sleep(std::time::Duration::from_millis(250));
        let second = s.sample();
        assert!(second.memory.total_bytes > 0);
        assert!(second.cpu.logical_cores > 0);
        assert!(second.process.pid > 0);
        assert!(second.process.rss_bytes > 0);
        assert!(second.network.iter().all(|n| n.rx_bytes_per_sec >= 0.0));
    }
}
