use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let root = repo_root()?;
    let app = env::var_os("POLYMARKET_APP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    let route_dir = root.join("route");
    let route_files = route_dir.join("files");
    let out_dir = app.join("app/polymarket");
    let tmp_dir = root.join("target/polymarket-v2-routes");
    let staging_dir = tmp_dir.join("staging/app/polymarket");

    require_tool("cargo")?;
    require_tool("wasm-tools")?;

    let routes = discover_routes(&route_files)?;
    if routes.is_empty() {
        return Err(format!(
            "no route files found under {}",
            route_files.display()
        ));
    }
    if env::args().nth(1).as_deref() == Some("check-caps") {
        let mut failures = Vec::new();
        for (route_path, source) in &routes {
            let artifact = out_dir.join(format!("{route_path}.wasm"));
            if let Err(err) = check_route_caps(route_path, source, &artifact) {
                failures.push(err);
            }
        }
        if failures.is_empty() {
            println!(
                "checked capability metadata for {} route components",
                routes.len()
            );
            return Ok(());
        }
        return Err(failures.join("\n"));
    }

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .map_err(|err| format!("remove {}: {err}", staging_dir.display()))?;
    }
    fs::create_dir_all(&tmp_dir).map_err(|err| format!("create {}: {err}", tmp_dir.display()))?;
    fs::create_dir_all(&staging_dir)
        .map_err(|err| format!("create {}: {err}", staging_dir.display()))?;

    for (route_path, source) in &routes {
        let route_id = route_id(route_path);
        let target_dir = tmp_dir.join("target");
        let core = target_dir.join("wasm32-unknown-unknown/release/bloom_polymarket_v2_route.wasm");
        let route_core = tmp_dir.join(format!("{route_id}.core.wasm"));
        let output = staging_dir.join(format!("{route_path}.wasm"));

        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("--manifest-path")
                .arg(route_dir.join("Cargo.toml"))
                .arg("--target")
                .arg("wasm32-unknown-unknown")
                .arg("--release")
                .arg("--target-dir")
                .arg(&target_dir)
                .env("BLOOM_ROUTE_RS", source)
                .env("BLOOM_ROUTE_PATH", route_path)
                .env(
                    "BLOOM_ROUTE_CANONICAL_PATH",
                    canonical_route_path(route_path),
                )
                .env("BLOOM_ROUTE_PARAMS", route_params(route_path))
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        )?;
        fs::copy(&core, &route_core)
            .map_err(|err| format!("copy {} to {}: {err}", core.display(), route_core.display()))?;

        let parent = output
            .parent()
            .ok_or_else(|| format!("output has no parent: {}", output.display()))?;
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;

        run_command(
            Command::new("wasm-tools")
                .arg("component")
                .arg("new")
                .arg(&route_core)
                .arg("-o")
                .arg(&output)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        )?;
        run_command(
            Command::new("wasm-tools")
                .arg("validate")
                .arg(&output)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit()),
        )?;
        check_route_caps(route_path, source, &output)?;
    }

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)
            .map_err(|err| format!("remove {}: {err}", out_dir.display()))?;
    }
    let out_parent = out_dir
        .parent()
        .ok_or_else(|| format!("output dir has no parent: {}", out_dir.display()))?;
    fs::create_dir_all(out_parent)
        .map_err(|err| format!("create {}: {err}", out_parent.display()))?;
    fs::rename(&staging_dir, &out_dir).map_err(|err| {
        format!(
            "replace {} with {}: {err}",
            out_dir.display(),
            staging_dir.display()
        )
    })?;

    println!(
        "wrote {} route components under {}",
        routes.len(),
        out_dir.display()
    );
    Ok(())
}

fn check_route_caps(route_path: &str, source: &Path, artifact: &Path) -> Result<(), String> {
    let required = required_caps(source)?;
    let wit = Command::new("wasm-tools")
        .arg("component")
        .arg("wit")
        .arg(artifact)
        .output()
        .map_err(|err| format!("inspect {}: {err}", artifact.display()))?;
    if !wit.status.success() {
        return Err(format!(
            "inspect {}: {}",
            artifact.display(),
            String::from_utf8_lossy(&wit.stderr).trim()
        ));
    }
    let imported = imported_caps(&String::from_utf8_lossy(&wit.stdout));
    let missing = required.difference(&imported).copied().collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "route {route_path} metadata requires caps absent from its artifact imports: {} (imports: {})",
            missing.join(", "),
            imported.into_iter().collect::<Vec<_>>().join(", ")
        ))
    }
}

fn required_caps(source: &Path) -> Result<BTreeSet<&'static str>, String> {
    let source =
        fs::read_to_string(source).map_err(|err| format!("read {}: {err}", source.display()))?;
    if let Some(start) = source.find(".caps(&[") {
        let rest = &source[start + ".caps(&[".len()..];
        let end = rest
            .find("])")
            .ok_or_else(|| format!("unterminated .caps override in {}", source))?;
        return Ok(ALL_CAPS
            .iter()
            .copied()
            .filter(|cap| rest[..end].contains(cap))
            .collect());
    }
    let caps: &[&str] = if source.contains("store_dir_spec()") {
        &["bloom:store", "bloom:vfs.read"]
    } else if source.contains("account_read_spec()") {
        &["bloom:http", "bloom:store", "bloom:vfs.read"]
    } else if source.contains("wallet_http_read_spec(") {
        &["bloom:http", "bloom:vfs.read"]
    } else if source.contains("http_dir_spec()") || source.contains("http_read_spec(") {
        &["bloom:http"]
    } else if source.contains("store_read_spec()") {
        &["bloom:store"]
    } else if source.contains("chain_read_spec()")
        || source.contains("write_spec()")
        || source.contains("signing_write_spec(")
    {
        ALL_CAPS
    } else {
        &[]
    };
    Ok(caps.iter().copied().collect())
}

const ALL_CAPS: &[&str] = &[
    "bloom:http",
    "bloom:store",
    "bloom:sign",
    "bloom:tx.outbox",
    "bloom:chain",
    "bloom:vfs.read",
    "bloom:vfs.write",
];

fn imported_caps(wit: &str) -> BTreeSet<&'static str> {
    let mut caps = BTreeSet::new();
    for line in wit.lines().map(str::trim) {
        if !line.starts_with("import ") {
            continue;
        }
        if line.contains("bloom:http/") {
            caps.insert("bloom:http");
        } else if line.contains("bloom:store/") {
            caps.insert("bloom:store");
        } else if line.contains("bloom:sign/") {
            caps.insert("bloom:sign");
        } else if line.contains("bloom:tx/outbox") {
            caps.insert("bloom:tx.outbox");
        } else if line.contains("bloom:chain/") {
            caps.insert("bloom:chain");
        } else if line.contains("bloom:vfs/readwrite") {
            // The component WIT interface import is narrowed by the functions
            // retained in its package block below.
            let block = wit.split("interface readwrite").nth(1).unwrap_or_default();
            if ["lookup:", "%list:", "read:"]
                .iter()
                .any(|name| block.contains(name))
            {
                caps.insert("bloom:vfs.read");
            }
            if block.contains("write:") {
                caps.insert("bloom:vfs.write");
            }
        }
    }
    caps
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR missing")?);
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent", manifest_dir.display()))
}

fn discover_routes(route_files: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut routes = BTreeMap::new();
    discover_routes_at(route_files, route_files, &mut routes)?;
    Ok(routes)
}

fn discover_routes_at(
    root: &Path,
    dir: &Path,
    routes: &mut BTreeMap<String, PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {} entry: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            discover_routes_at(root, &path, routes)?;
            continue;
        }
        if path.extension() != Some(OsStr::new("rs")) {
            continue;
        }
        if path.file_name() == Some(OsStr::new("$list.rs")) {
            return Err(format!(
                "{} is no longer supported; use $index.rs for directory routes",
                path.display()
            ));
        }
        let route_path = route_path(root, &path)?;
        if routes.insert(route_path.clone(), path.clone()).is_some() {
            return Err(format!("duplicate route path {route_path}"));
        }
    }
    Ok(())
}

fn route_path(root: &Path, source: &Path) -> Result<String, String> {
    let relative = source
        .strip_prefix(root)
        .map_err(|err| format!("strip {} from {}: {err}", root.display(), source.display()))?;
    let mut parts = Vec::new();
    for part in relative {
        let part = part
            .to_str()
            .ok_or_else(|| format!("non-utf8 route path {}", source.display()))?;
        parts.push(part.to_string());
    }
    let last = parts
        .last_mut()
        .ok_or_else(|| format!("empty route path {}", source.display()))?;
    *last = last
        .strip_suffix(".rs")
        .ok_or_else(|| format!("route source does not end in .rs: {}", source.display()))?
        .to_string();
    Ok(parts.join("/"))
}

fn route_id(route_path: &str) -> String {
    route_path
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect()
}

fn canonical_route_path(route_path: &str) -> String {
    match route_path {
        "$index" => String::new(),
        _ => route_path
            .strip_suffix("/$index")
            .unwrap_or(route_path)
            .to_string(),
    }
}

fn route_params(route_path: &str) -> String {
    route_path
        .split('/')
        .enumerate()
        .filter_map(|(index, segment)| {
            segment
                .strip_prefix('[')
                .and_then(|name| name.strip_suffix(']'))
                .map(|name| format!("{name}:{index}"))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn require_tool(tool: &str) -> Result<(), String> {
    let status = Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("run {tool} --version: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("missing required tool: {tool}"))
    }
}

fn run_command(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("run command: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed with status {status}"))
    }
}
