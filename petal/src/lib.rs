//! Foundational framework and Bloom host SDK for route petals.

pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "route-file",
        pub_export_macro: true,
        default_bindings_module: "petal::bindings",
        with: {
            "bloom:http/fetch@0.1.0": generate,
            "bloom:store/kv@0.1.0": generate,
            "bloom:sign/signing@0.1.0": generate,
            "bloom:chain/read@0.1.0": generate,
            "bloom:vfs/readwrite@0.1.0": generate,
            "bloom:env/runtime@0.1.0": generate,
        }
    });
}

fn component_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let bytes = sdk::random_bytes(buf.len()).map_err(|_| getrandom::Error::UNSUPPORTED)?;
    buf.copy_from_slice(&bytes);
    Ok(())
}

getrandom::register_custom_getrandom!(component_getrandom);

pub use bindings::{Ctx, Entry, Guest, RouteError, RouteMeta};
pub use bindings::bloom::route::types::EntryKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchResponse {
    Read(Vec<u8>),
    Write,
    Error { code: i32, message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignRequest {
    pub wallet: String,
    pub hash32: [u8; 32],
    pub purpose: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostStatus {
    NotFound,
    Denied,
    Invalid,
    Backend,
    BufferTooSmall { needed: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdkError {
    Host(HostStatus),
    Message(String),
}

impl SdkError {
    pub fn message(&self) -> String {
        match self {
            SdkError::Host(HostStatus::NotFound) => "not found".into(),
            SdkError::Host(HostStatus::Denied) => "denied".into(),
            SdkError::Host(HostStatus::Invalid) => "invalid".into(),
            SdkError::Host(HostStatus::Backend) => "backend error".into(),
            SdkError::Host(HostStatus::BufferTooSmall { needed }) => {
                format!("buffer too small: needs {needed} bytes")
            }
            SdkError::Message(message) => message.clone(),
        }
    }
}

pub mod sdk {
    pub use super::{DispatchResponse, HostStatus, HttpRequest, HttpResponse, SdkError, SignRequest};
    use crate::bindings::bloom::env::runtime as env;
    use crate::bindings::bloom::http::fetch as http;
    use crate::bindings::bloom::sign::signing as sign;
    use crate::bindings::bloom::store::kv as store;
    use crate::bindings::bloom::vfs::readwrite as vfs;

    const STATE_NS: &str = "state";
    const SECRET_NS: &str = "secrets";

    pub fn http_fetch(req: &HttpRequest, max_bytes: usize) -> Result<HttpResponse, SdkError> {
        let resp = http::fetch(&http::Request {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            body: req.body.clone(),
        })
        .map_err(host_err)?;
        if resp.body.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: resp.body.len(),
            }));
        }
        Ok(HttpResponse {
            status: resp.status,
            headers: resp.headers,
            body: resp.body,
        })
    }

    pub fn sign_hash(req: &SignRequest) -> Result<Vec<u8>, SdkError> {
        sign::sign_hash(&req.wallet, &req.hash32, &req.purpose).map_err(host_err)
    }

    pub fn store_get(key: &str, max_bytes: usize) -> Result<Vec<u8>, SdkError> {
        let namespace = namespace_for_key(key, false);
        let Some(bytes) = store::get(namespace, key).map_err(host_err)? else {
            return Err(SdkError::Host(HostStatus::NotFound));
        };
        if bytes.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: bytes.len(),
            }));
        }
        Ok(bytes)
    }

    pub fn store_put(key: &str, value: &[u8], secret: bool) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, secret);
        store::put(namespace, key, value, namespace == SECRET_NS).map_err(host_err)
    }

    pub fn store_put_new(key: &str, value: &[u8], secret: bool) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, secret);
        store::put_new(namespace, key, value, namespace == SECRET_NS).map_err(host_err)
    }

    pub fn store_del(key: &str) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, false);
        store::delete(namespace, key).map_err(host_err)
    }

    pub fn store_del_if_value(key: &str, expected: &[u8]) -> Result<(), SdkError> {
        let namespace = namespace_for_key(key, false);
        store::delete_if_value(namespace, key, expected).map_err(host_err)
    }

    pub fn store_list(prefix: &str, max_bytes: usize) -> Result<Vec<String>, SdkError> {
        let namespace = namespace_for_key(prefix, false);
        let keys = store::list(namespace, prefix).map_err(host_err)?;
        let size = keys.iter().map(|key| key.len()).sum::<usize>();
        if size > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall { needed: size }));
        }
        Ok(keys)
    }

    pub fn vfs_read(path: &str, max_bytes: usize) -> Result<Vec<u8>, SdkError> {
        let bytes = vfs::read(path).map_err(host_err)?;
        if bytes.len() > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall {
                needed: bytes.len(),
            }));
        }
        Ok(bytes)
    }

    pub fn vfs_write(path: &str, body: &[u8]) -> Result<(), SdkError> {
        vfs::write(path, body).map_err(host_err)
    }

    pub fn vfs_list(path: &str, max_bytes: usize) -> Result<Vec<String>, SdkError> {
        let _ = vfs::lookup(path).map_err(host_err)?;
        let entries = vfs::list(path).map_err(host_err)?;
        let size = entries.iter().map(|entry| entry.name.len()).sum::<usize>();
        if size > max_bytes {
            return Err(SdkError::Host(HostStatus::BufferTooSmall { needed: size }));
        }
        Ok(entries.into_iter().map(|entry| entry.name).collect())
    }

    pub fn now_ms() -> u64 {
        env::now_ms().unwrap_or(0)
    }

    pub fn random_bytes(len: usize) -> Result<Vec<u8>, SdkError> {
        let len = u32::try_from(len).map_err(|_| SdkError::Host(HostStatus::Invalid))?;
        env::random_bytes(len).map_err(host_err)
    }

    fn namespace_for_key(key: &str, secret: bool) -> &'static str {
        if secret || key.starts_with("creds/") {
            SECRET_NS
        } else {
            STATE_NS
        }
    }

    fn host_err(message: String) -> SdkError {
        let lower = message.to_ascii_lowercase();
        if lower.contains("not found") {
            SdkError::Host(HostStatus::NotFound)
        } else if lower.contains("denied") || lower.contains("permission") {
            SdkError::Host(HostStatus::Denied)
        } else if lower.contains("invalid") {
            SdkError::Host(HostStatus::Invalid)
        } else {
            SdkError::Message(message)
        }
    }
}

#[macro_export]
macro_rules! route_file {
    (spec: $spec:expr, list: $children:expr $(,)?) => {
        pub struct Route;

        impl $crate::Guest for Route {
            fn metadata(ctx: $crate::Ctx) -> Result<$crate::RouteMeta, $crate::RouteError> {
                $crate::framework_metadata(&ctx, $spec)
            }

            fn lookup(ctx: $crate::Ctx) -> Result<$crate::Entry, $crate::RouteError> {
                $crate::framework_lookup(&ctx, $spec)
            }

            fn list(ctx: $crate::Ctx) -> Result<Vec<$crate::Entry>, $crate::RouteError> {
                let children = $children;
                $crate::framework_list(&ctx, children)
            }

            fn read(_ctx: $crate::Ctx) -> Result<Vec<u8>, $crate::RouteError> {
                Err($crate::RouteError::Invalid("not a file".into()))
            }

            fn write(_ctx: $crate::Ctx, _body: Vec<u8>) -> Result<(), $crate::RouteError> {
                Err($crate::RouteError::Denied("path is not writable".into()))
            }
        }
    };
    (spec: $spec:expr, fallible_list: $children:expr $(,)?) => {
        pub struct Route;

        impl $crate::Guest for Route {
            fn metadata(ctx: $crate::Ctx) -> Result<$crate::RouteMeta, $crate::RouteError> {
                $crate::framework_metadata(&ctx, $spec)
            }

            fn lookup(ctx: $crate::Ctx) -> Result<$crate::Entry, $crate::RouteError> {
                $crate::framework_lookup(&ctx, $spec)
            }

            fn list(ctx: $crate::Ctx) -> Result<Vec<$crate::Entry>, $crate::RouteError> {
                let children = $children;
                $crate::framework_fallible_list(&ctx, children)
            }

            fn read(_ctx: $crate::Ctx) -> Result<Vec<u8>, $crate::RouteError> {
                Err($crate::RouteError::Invalid("not a file".into()))
            }

            fn write(_ctx: $crate::Ctx, _body: Vec<u8>) -> Result<(), $crate::RouteError> {
                Err($crate::RouteError::Denied("path is not writable".into()))
            }
        }
    };
    (spec: $spec:expr, ctx_list: $children:expr $(,)?) => {
        pub struct Route;

        impl $crate::Guest for Route {
            fn metadata(ctx: $crate::Ctx) -> Result<$crate::RouteMeta, $crate::RouteError> {
                $crate::framework_metadata(&ctx, $spec)
            }

            fn lookup(ctx: $crate::Ctx) -> Result<$crate::Entry, $crate::RouteError> {
                $crate::framework_lookup(&ctx, $spec)
            }

            fn list(ctx: $crate::Ctx) -> Result<Vec<$crate::Entry>, $crate::RouteError> {
                let children = $children;
                $crate::framework_fallible_list(&ctx, children(&ctx))
            }

            fn read(_ctx: $crate::Ctx) -> Result<Vec<u8>, $crate::RouteError> {
                Err($crate::RouteError::Invalid("not a file".into()))
            }

            fn write(_ctx: $crate::Ctx, _body: Vec<u8>) -> Result<(), $crate::RouteError> {
                Err($crate::RouteError::Denied("path is not writable".into()))
            }
        }
    };
    (spec: $spec:expr, read: $read:expr $(,)?) => {
        pub struct Route;

        impl $crate::Guest for Route {
            fn metadata(ctx: $crate::Ctx) -> Result<$crate::RouteMeta, $crate::RouteError> {
                $crate::framework_metadata(&ctx, $spec)
            }

            fn lookup(ctx: $crate::Ctx) -> Result<$crate::Entry, $crate::RouteError> {
                $crate::framework_lookup(&ctx, $spec)
            }

            fn list(_ctx: $crate::Ctx) -> Result<Vec<$crate::Entry>, $crate::RouteError> {
                Err($crate::RouteError::Invalid("not a directory".into()))
            }

            fn read(ctx: $crate::Ctx) -> Result<Vec<u8>, $crate::RouteError> {
                let read = $read;
                $crate::framework_read(read(&ctx))
            }

            fn write(_ctx: $crate::Ctx, _body: Vec<u8>) -> Result<(), $crate::RouteError> {
                Err($crate::RouteError::Denied("path is not writable".into()))
            }
        }
    };
    (spec: $spec:expr, read: $read:expr, write: $write:expr $(,)?) => {
        pub struct Route;

        impl $crate::Guest for Route {
            fn metadata(ctx: $crate::Ctx) -> Result<$crate::RouteMeta, $crate::RouteError> {
                $crate::framework_metadata(&ctx, $spec)
            }

            fn lookup(ctx: $crate::Ctx) -> Result<$crate::Entry, $crate::RouteError> {
                $crate::framework_lookup(&ctx, $spec)
            }

            fn list(_ctx: $crate::Ctx) -> Result<Vec<$crate::Entry>, $crate::RouteError> {
                Err($crate::RouteError::Invalid("not a directory".into()))
            }

            fn read(ctx: $crate::Ctx) -> Result<Vec<u8>, $crate::RouteError> {
                let read = $read;
                $crate::framework_read(read(&ctx))
            }

            fn write(ctx: $crate::Ctx, body: Vec<u8>) -> Result<(), $crate::RouteError> {
                let write = $write;
                $crate::framework_write(write(&ctx, &body))
            }
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteFileKind {
    Dir,
    File,
    WritableFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteSpec {
    kind: RouteFileKind,
    cache_ttl_ms: Option<u64>,
    side_effecting_read: bool,
    write_async: bool,
    required_caps: &'static [&'static str],
}

impl RouteSpec {
    const fn dir() -> Self {
        Self::new(RouteFileKind::Dir)
    }

    const fn file() -> Self {
        Self::new(RouteFileKind::File)
    }

    const fn writable() -> Self {
        Self::new(RouteFileKind::WritableFile)
    }

    const fn new(kind: RouteFileKind) -> Self {
        Self {
            kind,
            cache_ttl_ms: Some(30_000),
            side_effecting_read: false,
            write_async: false,
            required_caps: CAPS_NONE,
        }
    }

    const fn caps(mut self, caps: &'static [&'static str]) -> Self {
        self.required_caps = caps;
        self
    }

    const fn ttl(mut self, ttl: Option<u64>) -> Self {
        self.cache_ttl_ms = ttl;
        self
    }

    const fn side_effecting_read(mut self, value: bool) -> Self {
        self.side_effecting_read = value;
        self
    }
}

const CAPS_NONE: &[&str] = &[];
const CAPS_HTTP: &[&str] = &["bloom:http"];
const CAPS_STORE: &[&str] = &["bloom:store"];
const CAPS_STORE_VFS_READ: &[&str] = &["bloom:store", "bloom:vfs.read"];
const CAPS_HTTP_VFS_READ: &[&str] = &["bloom:http", "bloom:vfs.read"];
const CAPS_HTTP_STORE_VFS_READ: &[&str] = &["bloom:http", "bloom:store", "bloom:vfs.read"];
const CAPS_HTTP_STORE_SIGN_VFS: &[&str] = &[
    "bloom:http",
    "bloom:store",
    "bloom:sign",
    "bloom:vfs.read",
    "bloom:vfs.write",
];

pub fn static_dir_spec() -> RouteSpec {
    RouteSpec::dir()
}

pub fn store_dir_spec() -> RouteSpec {
    RouteSpec::dir().caps(CAPS_STORE_VFS_READ)
}

pub fn http_dir_spec() -> RouteSpec {
    RouteSpec::dir().caps(CAPS_HTTP)
}

pub fn static_read_spec() -> RouteSpec {
    RouteSpec::file()
}

pub fn http_read_spec(ttl_ms: u64) -> RouteSpec {
    RouteSpec::file().caps(CAPS_HTTP).ttl(Some(ttl_ms))
}

pub fn store_read_spec() -> RouteSpec {
    RouteSpec::file().caps(CAPS_STORE)
}

pub fn wallet_http_read_spec(ttl_ms: u64) -> RouteSpec {
    RouteSpec::file().caps(CAPS_HTTP_VFS_READ).ttl(Some(ttl_ms))
}

pub fn account_read_spec() -> RouteSpec {
    RouteSpec::file()
        .caps(CAPS_HTTP_STORE_VFS_READ)
        .ttl(Some(5_000))
}

pub fn chain_read_spec() -> RouteSpec {
    RouteSpec::file()
        .caps(CAPS_HTTP_STORE_SIGN_VFS)
        .ttl(None)
        .side_effecting_read(true)
}

pub fn write_spec() -> RouteSpec {
    RouteSpec::writable()
        .caps(CAPS_HTTP_STORE_SIGN_VFS)
        .ttl(None)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteChild {
    name: String,
    kind: RouteFileKind,
}

pub fn current_route_path() -> &'static str {
    env!("BLOOM_ROUTE_PATH")
}

pub fn current_route_canonical_path() -> &'static str {
    env!("BLOOM_ROUTE_CANONICAL_PATH")
}

pub fn framework_metadata(_ctx: &Ctx, spec: RouteSpec) -> Result<RouteMeta, RouteError> {
    Ok(RouteMeta {
        kind: match spec.kind {
            RouteFileKind::Dir => EntryKind::Dir,
            RouteFileKind::File | RouteFileKind::WritableFile => EntryKind::File,
        },
        mode: match spec.kind {
            RouteFileKind::Dir => 0o755,
            RouteFileKind::File => 0o444,
            RouteFileKind::WritableFile => 0o644,
        },
        cache_ttl_ms: spec.cache_ttl_ms,
        side_effecting_read: spec.side_effecting_read,
        write_async: spec.write_async,
        description: Some(format!("Petal route {}", current_route_path())),
        consent_summary: None,
        required_caps: spec
            .required_caps
            .iter()
            .map(|cap| (*cap).to_string())
            .collect(),
        sign_intent: None,
        executable: false,
    })
}

pub fn framework_lookup(ctx: &Ctx, spec: RouteSpec) -> Result<Entry, RouteError> {
    let relative = route_relative(ctx);
    Ok(framework_entry(entry_name(&relative), spec.kind))
}

pub fn framework_list(_ctx: &Ctx, children: Vec<RouteChild>) -> Result<Vec<Entry>, RouteError> {
    Ok(children
        .into_iter()
        .filter(|child| is_safe_segment(&child.name))
        .map(|child| framework_entry(&child.name, child.kind))
        .collect())
}

pub fn framework_fallible_list(
    ctx: &Ctx,
    children: Result<Vec<RouteChild>, DispatchResponse>,
) -> Result<Vec<Entry>, RouteError> {
    match children {
        Ok(children) => framework_list(ctx, children),
        Err(DispatchResponse::Error { code, message }) => Err(route_error(code, message)),
        Err(_) => Err(RouteError::Backend(
            "list returned non-list response".into(),
        )),
    }
}

pub fn framework_read(resp: DispatchResponse) -> Result<Vec<u8>, RouteError> {
    match resp {
        DispatchResponse::Read(bytes) => Ok(bytes),
        DispatchResponse::Error { code, message } => Err(route_error(code, message)),
        _ => Err(RouteError::Backend(
            "read returned non-read response".into(),
        )),
    }
}

pub fn framework_write(resp: DispatchResponse) -> Result<(), RouteError> {
    match resp {
        DispatchResponse::Write => Ok(()),
        DispatchResponse::Error { code, message } => Err(route_error(code, message)),
        _ => Err(RouteError::Backend(
            "write returned non-write response".into(),
        )),
    }
}

pub fn route_relative(ctx: &Ctx) -> String {
    if ctx.path.is_empty() {
        return current_route_canonical_path().to_string();
    }
    metadata_path(&ctx.path)
}

pub fn route_param<'a>(ctx: &'a Ctx, name: &str) -> Option<&'a str> {
    ctx.params
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value.as_str()))
}

pub fn route_segment<'a>(ctx: &'a Ctx, index: usize) -> Option<&'a str> {
    split(&ctx.path).get(index).copied()
}

pub fn param<'a>(ctx: &'a Ctx, name: &str) -> Result<&'a str, DispatchResponse> {
    route_param(ctx, name)
        .or_else(|| route_generated_param(ctx, name))
        .ok_or_else(|| route_invalid(format!("missing {name}")))
}

pub fn route_generated_param<'a>(ctx: &'a Ctx, name: &str) -> Option<&'a str> {
    for pair in env!("BLOOM_ROUTE_PARAMS").split(',') {
        let Some((candidate, index)) = pair.split_once(':') else {
            continue;
        };
        if candidate != name {
            continue;
        }
        let Ok(index) = index.parse::<usize>() else {
            continue;
        };
        return route_segment(ctx, index);
    }
    None
}

pub fn route_invalid(message: impl Into<String>) -> DispatchResponse {
    error(-3, message)
}

pub fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !segment.bytes().any(|byte| byte == 0)
}

pub fn split(relative: &str) -> Vec<&str> {
    if relative.is_empty() {
        Vec::new()
    } else {
        relative.split('/').collect()
    }
}

pub fn entry_name(relative: &str) -> &str {
    relative
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("")
}

pub fn error(code: i32, message: impl Into<String>) -> DispatchResponse {
    DispatchResponse::Error {
        code,
        message: message.into(),
    }
}

pub fn dir(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::Dir,
    }
}

pub fn file(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::File,
    }
}

pub fn writable(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::WritableFile,
    }
}

pub fn dirs(names: Vec<String>) -> Vec<RouteChild> {
    names.into_iter().map(dir).collect()
}

pub fn dir_names(names: &[&str]) -> Vec<RouteChild> {
    names.iter().map(|name| dir(*name)).collect()
}

pub fn files(names: &[&str]) -> Vec<RouteChild> {
    names.iter().map(|name| file(*name)).collect()
}

pub fn result_dirs(
    names: Result<Vec<String>, DispatchResponse>,
) -> Result<Vec<RouteChild>, DispatchResponse> {
    names.map(dirs)
}

pub fn framework_entry(name: &str, kind: RouteFileKind) -> Entry {
    Entry {
        name: name.into(),
        kind: match kind {
            RouteFileKind::Dir => EntryKind::Dir,
            RouteFileKind::File | RouteFileKind::WritableFile => EntryKind::File,
        },
        mode: match kind {
            RouteFileKind::Dir => 0o755,
            RouteFileKind::File => 0o444,
            RouteFileKind::WritableFile => 0o644,
        },
        size: Some(0),
        link_target: None,
    }
}

pub fn metadata_path(path: &str) -> String {
    match path {
        "$index" => String::new(),
        _ => path.strip_suffix("/$index").unwrap_or(path).to_string(),
    }
}

pub fn route_error(code: i32, message: String) -> RouteError {
    match code {
        -1 => RouteError::NotFound(message),
        -2 => RouteError::Denied(message),
        -3 => RouteError::Invalid(message),
        -4 => RouteError::Backend(message),
        _ => RouteError::Unsupported(message),
    }
}

pub fn read_json_value<T: serde::Serialize>(value: &T) -> DispatchResponse {
    match serde_json::to_vec_pretty(value) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(e) => error(-4, e.to_string()),
    }
}
