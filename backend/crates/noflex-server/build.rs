// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const FALLBACK: &str = r#"<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>NoFlex</title></head><body><main><h1>NoFlex</h1><p>The web application has not been built. Run <code>npm run build</code> in <code>web/</code>, then rebuild the server.</p></main></body></html>"#;

fn main() {
    println!("cargo:rerun-if-env-changed=NOFLEX_WEB_DIR");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let web_dir = env::var_os("NOFLEX_WEB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../../../web/build"));
    // A nonexistent rerun-if-changed path is always stale, which would
    // rerun this script (and rebuild the crate) on every build. Without
    // a build, watch the web package manifest instead; a fresh
    // `npm run build` then needs NOFLEX_WEB_DIR or a touch of it.
    let watched = if web_dir.is_dir() {
        web_dir.clone()
    } else {
        manifest_dir.join("../../../web/package.json")
    };
    if watched.exists() {
        match watched.to_str() {
            Some(path) => println!("cargo:rerun-if-changed={path}"),
            None => println!(
                "cargo:warning=NoFlex web build path is not UTF-8; changes will not be tracked"
            ),
        }
    } else {
        // Nothing to watch: track only this script so cargo doesn't fall
        // back to rescanning the whole package on every build.
        println!("cargo:rerun-if-changed=build.rs");
    }

    let mut assets = Vec::new();
    let index = if web_dir.join("index.html").is_file() {
        collect_assets(&web_dir, &web_dir, &mut assets).expect("read web build");
        assets.sort_by(|left, right| left.0.cmp(&right.0));
        match assets
            .iter()
            .find(|(path, _)| path == "index.html")
            .and_then(|(_, path)| fs::read_to_string(path).ok())
        {
            Some(index) => index,
            None => {
                println!(
                    "cargo:warning=NoFlex web build at {} has no UTF-8 index.html; embedding the development fallback",
                    web_dir.display()
                );
                assets.clear();
                FALLBACK.to_owned()
            }
        }
    } else {
        println!(
            "cargo:warning=NoFlex web build not found at {}; embedding the development fallback (after `npm run build`, touch web/package.json or set NOFLEX_WEB_DIR to re-embed)",
            web_dir.display()
        );
        FALLBACK.to_owned()
    };
    let csp = content_security_policy(&index);

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let embedded_assets = output.join("web-assets");
    fs::create_dir_all(&embedded_assets).expect("create embedded asset directory");

    let mut generated = String::from("// SPDX-License-Identifier: AGPL-3.0-only\n\n");
    generated.push_str(&format!(
        "#[cfg(test)]\npub const WEB_BUILD_PRESENT: bool = {};\n",
        !assets.is_empty()
    ));
    generated.push_str(&format!(
        "pub const CONTENT_SECURITY_POLICY: &str = {csp:?};\n"
    ));
    generated.push_str("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    if assets.is_empty() {
        generated.push_str(&format!(
            "    (\"index.html\", {:?}.as_bytes()),\n",
            FALLBACK
        ));
    } else {
        for (index, (relative, absolute)) in assets.into_iter().enumerate() {
            let destination = embedded_assets.join(index.to_string());
            fs::copy(absolute, destination).expect("copy embedded web asset");
            generated.push_str(&format!(
                "    ({relative:?}, include_bytes!(\"web-assets/{index}\")),\n"
            ));
        }
    }
    generated.push_str("];\n");

    fs::write(output.join("web_assets.rs"), generated).expect("write embedded web module");
}

fn collect_assets(
    root: &Path,
    directory: &Path,
    assets: &mut Vec<(String, PathBuf)>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_assets(root, &path, assets)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("asset remains below web root");
            let Some(relative) = relative.to_str() else {
                println!(
                    "cargo:warning=Skipping NoFlex web asset with a non-UTF-8 path: {}",
                    path.display()
                );
                continue;
            };
            assets.push((relative.replace('\\', "/"), path));
        }
    }
    Ok(())
}

fn content_security_policy(index: &str) -> String {
    let mut script_sources = String::from("'self'");
    for body in inline_elements(index, "script") {
        script_sources.push_str(" 'sha256-");
        script_sources.push_str(&base64(&Sha256::digest(body.as_bytes())));
        script_sources.push('\'');
    }

    let mut style_sources = String::from("'self'");
    for body in inline_elements(index, "style") {
        push_hash(&mut style_sources, body);
    }
    format!(
        "default-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'; script-src {script_sources}; style-src {style_sources}; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; font-src 'self'"
    )
}

fn push_hash(sources: &mut String, content: &str) {
    sources.push_str(" 'sha256-");
    sources.push_str(&base64(&Sha256::digest(content.as_bytes())));
    sources.push('\'');
}

fn inline_elements<'a>(html: &'a str, tag: &str) -> Vec<&'a str> {
    let mut bodies = Vec::new();
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    let mut remainder = html;
    while let Some(start) = remainder.find(&opening) {
        remainder = &remainder[start + opening.len()..];
        let Some(tag_end) = remainder.find('>') else {
            break;
        };
        let attributes = &remainder[..tag_end];
        remainder = &remainder[tag_end + 1..];
        let Some(end) = remainder.find(&closing) else {
            break;
        };
        if !attributes
            .split_ascii_whitespace()
            .any(|attribute| attribute.starts_with("src="))
        {
            bodies.push(&remainder[..end]);
        }
        remainder = &remainder[end + closing.len()..];
    }
    bodies
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}
