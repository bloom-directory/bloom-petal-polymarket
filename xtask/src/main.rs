use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ALL_CAPS: &[&str] = &[
    "bloom:http",
    "bloom:store",
    "bloom:sign",
    "bloom:tx.outbox",
    "bloom:chain",
    "bloom:vfs.read",
    "bloom:vfs.write",
];

#[derive(Clone, Debug)]
struct Route {
    path: String,
    canonical: String,
    source: PathBuf,
    package: String,
}

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
    let routes = discover_routes(&root.join("route/files"))?;
    if routes.is_empty() {
        return Err("no route controllers found".into());
    }
    let out_dir = app.join("app/polymarket");

    if env::args().nth(1).as_deref() == Some("check-caps") {
        let failures = routes
            .iter()
            .filter_map(|route| {
                check_route_caps(route, &out_dir.join(format!("{}.wasm", route.path))).err()
            })
            .collect::<Vec<_>>();
        if failures.is_empty() {
            println!(
                "checked capability metadata for {} route components",
                routes.len()
            );
            return Ok(());
        }
        return Err(failures.join("\n"));
    }

    require_tool("cargo")?;
    require_tool("wasm-tools")?;
    let build_root = root.join("target/polymarket-v2-routes");
    let workspace = build_root.join("workspace");
    generate_workspace(&root, &workspace, &routes)?;
    generate_lockfile(&workspace)?;
    let artifacts = build_workspace(&workspace, &routes)?;

    let staging = build_root.join("staging/app/polymarket");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(display_err("remove", &staging))?;
    }
    fs::create_dir_all(&staging).map_err(display_err("create", &staging))?;
    let mut failures = Vec::new();
    for route in &routes {
        let result = (|| {
            let core = artifacts
                .get(&route.package)
                .ok_or_else(|| format!("no artifact for {}", route.package))?;
            let output = staging.join(format!("{}.wasm", route.path));
            fs::create_dir_all(output.parent().unwrap())
                .map_err(display_err("create", output.parent().unwrap()))?;
            command(
                Command::new("wasm-tools")
                    .args(["component", "new"])
                    .arg(core)
                    .arg("-o")
                    .arg(&output),
            )?;
            command(Command::new("wasm-tools").arg("validate").arg(&output))?;
            check_route_caps(route, &output)
        })();
        if let Err(err) = result {
            failures.push(err);
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    let staged = discover_wasm_paths(&staging)?;
    let expected = routes
        .iter()
        .map(|r| format!("{}.wasm", r.path))
        .collect::<BTreeSet<_>>();
    if staged != expected {
        return Err("staging route set differs from discovered controllers".into());
    }

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir).map_err(display_err("remove", &out_dir))?;
    }
    fs::create_dir_all(out_dir.parent().unwrap())
        .map_err(display_err("create", out_dir.parent().unwrap()))?;
    fs::rename(&staging, &out_dir).map_err(|e| format!("install {}: {e}", out_dir.display()))?;
    println!(
        "wrote {} route components under {}",
        routes.len(),
        out_dir.display()
    );
    Ok(())
}

fn generate_workspace(root: &Path, workspace: &Path, routes: &[Route]) -> Result<(), String> {
    fs::create_dir_all(workspace).map_err(display_err("create", workspace))?;
    let members = routes
        .iter()
        .map(|r| format!("    \"members/{}\",", r.package))
        .collect::<Vec<_>>()
        .join("\n");
    write_if_changed(
        &workspace.join("Cargo.toml"),
        &format!(
            "[workspace]\nresolver = \"2\"\nmembers = [\n{members}\n]\n\n[profile.release]\nopt-level = 3\ndebug = false\nstrip = \"none\"\ndebug-assertions = false\noverflow-checks = false\nlto = false\npanic = \"unwind\"\nincremental = false\ncodegen-units = 16\nrpath = false\n"
        ),
    )?;
    let wanted = routes
        .iter()
        .map(|r| r.package.as_str())
        .collect::<BTreeSet<_>>();
    let members_dir = workspace.join("members");
    fs::create_dir_all(&members_dir).map_err(display_err("create", &members_dir))?;
    for entry in fs::read_dir(&members_dir).map_err(display_err("read", &members_dir))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() && !wanted.contains(path.file_name().and_then(OsStr::to_str).unwrap_or(""))
        {
            fs::remove_dir_all(&path).map_err(display_err("prune", &path))?;
        }
    }
    for route in routes {
        let dir = members_dir.join(&route.package);
        let manifest = format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nalloy = {{ version = \"2\", default-features = false, features = [\"dyn-abi\", \"k256\", \"signer-local\", \"sol-types\", \"std\"] }}\nhex = \"0.4\"\npetal = {{ path = \"../../../../../petal\" }}\npolymarket_route = {{ package = \"bloom-polymarket-v2-route\", path = \"../../../../../route\" }}\nserde_json = \"1\"\n",
            route.package
        );
        let params = route_params(&route.path)
            .into_iter()
            .map(|(n, i)| format!("        ({n:?}, {i}),"))
            .collect::<Vec<_>>()
            .join("\n");
        let relative_source = route
            .source
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let source = format!(
            "#![allow(clippy::too_many_arguments)]\n#![allow(dead_code, clippy::upper_case_acronyms)]\n\npub struct __PetalRouteIdentity;\nimpl petal::RouteIdentity for __PetalRouteIdentity {{\n    const PATH: &'static str = {:?};\n    const CANONICAL_PATH: &'static str = {:?};\n    const PARAMS: &'static [(&'static str, usize)] = &[\n{}\n    ];\n}}\n\npub use polymarket_route::*;\n\nmod selected_route {{\n    include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../../../{}\"));\n}}\n\nuse selected_route::Route;\npetal::bindings::export!(Route);\n",
            route.path, route.canonical, params, relative_source
        );
        write_if_changed(&dir.join("Cargo.toml"), &manifest)?;
        write_if_changed(&dir.join("src/lib.rs"), &source)?;
    }
    Ok(())
}

fn generate_lockfile(workspace: &Path) -> Result<(), String> {
    command(
        Command::new("cargo")
            .arg("generate-lockfile")
            .arg("--offline")
            .arg("--manifest-path")
            .arg(workspace.join("Cargo.toml")),
    )
}

fn build_workspace(
    workspace: &Path,
    routes: &[Route],
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "--workspace",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
        "--locked",
        "--message-format=json-render-diagnostics",
    ])
    .arg("--manifest-path")
    .arg(workspace.join("Cargo.toml"))
    .arg("--target-dir")
    .arg(workspace.parent().unwrap().join("target"))
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());
    if let Some(jobs) = env::var_os("POLYMARKET_BUILD_JOBS") {
        cmd.arg("--jobs").arg(jobs);
    }
    let mut child = cmd.spawn().map_err(|e| format!("run cargo build: {e}"))?;
    let stdout = child.stdout.take().ok_or("cargo stdout unavailable")?;
    let wanted = routes
        .iter()
        .map(|r| r.package.as_str())
        .collect::<BTreeSet<_>>();
    let mut artifacts: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value["reason"] != "compiler-artifact"
            || !value["target"]["kind"]
                .as_array()
                .is_some_and(|a| a.iter().any(|k| k == "cdylib"))
        {
            continue;
        }
        let Some(name) = value["target"]["name"].as_str() else {
            continue;
        };
        let package = name.replace('_', "-");
        if !wanted.contains(package.as_str()) {
            continue;
        }
        for filename in value["filenames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            if filename.ends_with(".wasm") {
                artifacts
                    .entry(package.clone())
                    .or_default()
                    .push(filename.into());
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo workspace build failed with {status}"));
    }
    let mut exact = BTreeMap::new();
    for route in routes {
        let files = artifacts.remove(&route.package).unwrap_or_default();
        if files.len() != 1 {
            return Err(format!(
                "{} produced {} core WASMs",
                route.package,
                files.len()
            ));
        }
        exact.insert(route.package.clone(), files[0].clone());
    }
    Ok(exact)
}

fn discover_routes(root: &Path) -> Result<Vec<Route>, String> {
    let mut sources = Vec::new();
    discover_at(root, root, &mut sources)?;
    sources.sort();
    let mut routes = Vec::new();
    let mut packages = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for source in sources {
        let path = route_path(root, &source)?;
        let canonical = canonical_route_path(&path);
        let package = package_name(&path);
        if !paths.insert(path.clone()) || !packages.insert(package.clone()) {
            return Err(format!("route identity collision for {path}"));
        }
        routes.push(Route {
            path,
            canonical,
            source,
            package,
        });
    }
    Ok(routes)
}

fn discover_at(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(display_err("read", dir))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            discover_at(root, &path, out)?;
        } else if path.extension() == Some(OsStr::new("rs")) {
            if path.file_name() == Some(OsStr::new("$list.rs")) {
                return Err(format!("{} is unsupported; use $index.rs", path.display()));
            }
            out.push(path);
        }
    }
    let _ = root;
    Ok(())
}

fn route_path(root: &Path, source: &Path) -> Result<String, String> {
    let mut path = source
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    path.truncate(path.len() - 3);
    Ok(path)
}
fn canonical_route_path(path: &str) -> String {
    if path == "$index" {
        "".into()
    } else {
        path.strip_suffix("/$index").unwrap_or(path).into()
    }
}
fn route_params(path: &str) -> Vec<(&str, usize)> {
    path.split('/')
        .enumerate()
        .filter_map(|(i, s)| {
            s.strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .map(|n| (n, i))
        })
        .collect()
}
fn package_name(path: &str) -> String {
    let readable = path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let readable = readable
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let hash = blake3::hash(path.as_bytes()).to_hex();
    format!(
        "polymarket-route-{}-{}",
        if readable.is_empty() {
            "root"
        } else {
            &readable
        },
        &hash[..10]
    )
}

fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    fs::create_dir_all(path.parent().unwrap())
        .map_err(display_err("create", path.parent().unwrap()))?;
    fs::write(path, content).map_err(display_err("write", path))
}

fn check_route_caps(route: &Route, artifact: &Path) -> Result<(), String> {
    let required = required_caps(&route.source)?;
    let output = Command::new("wasm-tools")
        .args(["component", "wit"])
        .arg(artifact)
        .output()
        .map_err(|e| format!("inspect {}: {e}", artifact.display()))?;
    if !output.status.success() {
        return Err(format!(
            "inspect {}: {}",
            artifact.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let imported = imported_caps(&String::from_utf8_lossy(&output.stdout));
    let missing = required.difference(&imported).copied().collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "route {} requires absent imports: {}",
            route.path,
            missing.join(", ")
        ))
    }
}

fn required_caps(source: &Path) -> Result<BTreeSet<&'static str>, String> {
    let source = fs::read_to_string(source).map_err(display_err("read", source))?;
    if let Some(start) = source.find(".caps(&[") {
        let rest = &source[start + 8..];
        let end = rest.find("])").ok_or("unterminated caps override")?;
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

fn imported_caps(wit: &str) -> BTreeSet<&'static str> {
    let mut caps = BTreeSet::new();
    for line in wit
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("import "))
    {
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
            let block = wit.split("interface readwrite").nth(1).unwrap_or_default();
            if ["lookup:", "%list:", "read:"]
                .iter()
                .any(|n| block.contains(n))
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

fn discover_wasm_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(display_err("read", dir))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else if path.extension() == Some(OsStr::new("wasm")) {
                out.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeSet::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn repo_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or("xtask has no parent".into())
}
fn require_tool(tool: &str) -> Result<(), String> {
    command(
        Command::new(tool)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
}
fn command(command: &mut Command) -> Result<(), String> {
    let status = command.status().map_err(|e| format!("run command: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed with {status}"))
    }
}
fn display_err<'a>(action: &'a str, path: &'a Path) -> impl FnOnce(std::io::Error) -> String + 'a {
    move |e| format!("{action} {}: {e}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonicalizes_indexes() {
        assert_eq!(canonical_route_path("$index"), "");
        assert_eq!(canonical_route_path("a/$index"), "a");
    }
    #[test]
    fn extracts_typed_params() {
        assert_eq!(
            route_params("trade/[wallet]/[id]/file.json"),
            vec![("wallet", 1), ("id", 2)]
        );
    }
    #[test]
    fn package_names_are_stable_and_collision_safe() {
        assert_eq!(package_name("a[b]"), package_name("a[b]"));
        assert_ne!(package_name("a[b]"), package_name("a-b"));
    }
}
