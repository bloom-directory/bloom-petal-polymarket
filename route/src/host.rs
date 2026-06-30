fn component_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let bytes =
        bloom_petal_sdk::random_bytes(buf.len()).map_err(|_| getrandom::Error::UNSUPPORTED)?;
    buf.copy_from_slice(&bytes);
    Ok(())
}

getrandom::register_custom_getrandom!(component_getrandom);

pub(crate) mod bloom_petal_sdk {
    use crate::bloom::env::runtime as env;
    use crate::bloom::http::fetch as http;
    use crate::bloom::sign::signing as sign;
    use crate::bloom::store::kv as store;
    use crate::bloom::vfs::readwrite as vfs;

    const STATE_NS: &str = "state";
    const SECRET_NS: &str = "secrets";

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

    #[allow(dead_code)]
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
