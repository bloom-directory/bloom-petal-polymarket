use crate::bloom_petal_sdk::DispatchResponse;
use crate::{Ctx, Entry, EntryKind, Result, RouteError, RouteMeta};

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
pub(crate) enum RouteFileKind {
    Dir,
    File,
    WritableFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteSpec {
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

pub(crate) fn static_dir_spec() -> RouteSpec {
    RouteSpec::dir()
}

pub(crate) fn store_dir_spec() -> RouteSpec {
    RouteSpec::dir().caps(CAPS_STORE_VFS_READ)
}

pub(crate) fn http_dir_spec() -> RouteSpec {
    RouteSpec::dir().caps(CAPS_HTTP)
}

pub(crate) fn static_read_spec() -> RouteSpec {
    RouteSpec::file()
}

pub(crate) fn http_read_spec(ttl_ms: u64) -> RouteSpec {
    RouteSpec::file().caps(CAPS_HTTP).ttl(Some(ttl_ms))
}

pub(crate) fn store_read_spec() -> RouteSpec {
    RouteSpec::file().caps(CAPS_STORE)
}

pub(crate) fn wallet_http_read_spec(ttl_ms: u64) -> RouteSpec {
    RouteSpec::file().caps(CAPS_HTTP_VFS_READ).ttl(Some(ttl_ms))
}

pub(crate) fn account_read_spec() -> RouteSpec {
    RouteSpec::file()
        .caps(CAPS_HTTP_STORE_VFS_READ)
        .ttl(Some(5_000))
}

pub(crate) fn chain_read_spec() -> RouteSpec {
    RouteSpec::file()
        .caps(CAPS_HTTP_STORE_SIGN_VFS)
        .ttl(None)
        .side_effecting_read(true)
}

pub(crate) fn write_spec() -> RouteSpec {
    RouteSpec::writable()
        .caps(CAPS_HTTP_STORE_SIGN_VFS)
        .ttl(None)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteChild {
    name: String,
    kind: RouteFileKind,
}

pub(crate) fn current_route_path() -> &'static str {
    env!("BLOOM_ROUTE_PATH")
}

pub(crate) fn current_route_canonical_path() -> &'static str {
    env!("BLOOM_ROUTE_CANONICAL_PATH")
}

pub(crate) fn framework_metadata(_ctx: &Ctx, spec: RouteSpec) -> Result<RouteMeta, RouteError> {
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
        description: Some(format!("Polymarket route {}", current_route_path())),
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

pub(crate) fn framework_lookup(ctx: &Ctx, spec: RouteSpec) -> Result<Entry, RouteError> {
    let relative = route_relative(ctx);
    Ok(framework_entry(entry_name(&relative), spec.kind))
}

pub(crate) fn framework_list(
    _ctx: &Ctx,
    children: Vec<RouteChild>,
) -> Result<Vec<Entry>, RouteError> {
    Ok(children
        .into_iter()
        .filter(|child| is_safe_segment(&child.name))
        .map(|child| framework_entry(&child.name, child.kind))
        .collect())
}

pub(crate) fn framework_fallible_list(
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

pub(crate) fn framework_read(resp: DispatchResponse) -> Result<Vec<u8>, RouteError> {
    match resp {
        DispatchResponse::Read(bytes) => Ok(bytes),
        DispatchResponse::Error { code, message } => Err(route_error(code, message)),
        _ => Err(RouteError::Backend(
            "read returned non-read response".into(),
        )),
    }
}

pub(crate) fn framework_write(resp: DispatchResponse) -> Result<(), RouteError> {
    match resp {
        DispatchResponse::Write => Ok(()),
        DispatchResponse::Error { code, message } => Err(route_error(code, message)),
        _ => Err(RouteError::Backend(
            "write returned non-write response".into(),
        )),
    }
}

pub(crate) fn route_relative(ctx: &Ctx) -> String {
    if ctx.path.is_empty() {
        return current_route_canonical_path().to_string();
    }
    metadata_path(&ctx.path)
}

pub(crate) fn route_param<'a>(ctx: &'a Ctx, name: &str) -> Option<&'a str> {
    ctx.params
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value.as_str()))
}

pub(crate) fn route_segment<'a>(ctx: &'a Ctx, index: usize) -> Option<&'a str> {
    split(&ctx.path).get(index).copied()
}

pub(crate) fn param<'a>(ctx: &'a Ctx, name: &str) -> Result<&'a str, DispatchResponse> {
    route_param(ctx, name)
        .or_else(|| route_generated_param(ctx, name))
        .ok_or_else(|| route_invalid(format!("missing {name}")))
}

pub(crate) fn route_generated_param<'a>(ctx: &'a Ctx, name: &str) -> Option<&'a str> {
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

pub(crate) fn route_invalid(message: impl Into<String>) -> DispatchResponse {
    error(-3, message)
}

pub(crate) fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !segment.bytes().any(|byte| byte == 0)
}

pub(crate) fn split(relative: &str) -> Vec<&str> {
    if relative.is_empty() {
        Vec::new()
    } else {
        relative.split('/').collect()
    }
}

pub(crate) fn entry_name(relative: &str) -> &str {
    relative
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("")
}

pub(crate) fn error(code: i32, message: impl Into<String>) -> DispatchResponse {
    DispatchResponse::Error {
        code,
        message: message.into(),
    }
}

pub(crate) fn dir(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::Dir,
    }
}

pub(crate) fn file(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::File,
    }
}

pub(crate) fn writable(name: impl Into<String>) -> RouteChild {
    RouteChild {
        name: name.into(),
        kind: RouteFileKind::WritableFile,
    }
}

pub(crate) fn dirs(names: Vec<String>) -> Vec<RouteChild> {
    names.into_iter().map(dir).collect()
}

pub(crate) fn dir_names(names: &[&str]) -> Vec<RouteChild> {
    names.iter().map(|name| dir(*name)).collect()
}

pub(crate) fn files(names: &[&str]) -> Vec<RouteChild> {
    names.iter().map(|name| file(*name)).collect()
}

pub(crate) fn result_dirs(
    names: Result<Vec<String>, DispatchResponse>,
) -> Result<Vec<RouteChild>, DispatchResponse> {
    names.map(dirs)
}

pub(crate) fn framework_entry(name: &str, kind: RouteFileKind) -> Entry {
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

pub(crate) fn metadata_path(path: &str) -> String {
    match path {
        "$index" => String::new(),
        _ => path.strip_suffix("/$index").unwrap_or(path).to_string(),
    }
}

pub(crate) fn route_error(code: i32, message: String) -> RouteError {
    match code {
        -1 => RouteError::NotFound(message),
        -2 => RouteError::Denied(message),
        -3 => RouteError::Invalid(message),
        -4 => RouteError::Backend(message),
        _ => RouteError::Unsupported(message),
    }
}
