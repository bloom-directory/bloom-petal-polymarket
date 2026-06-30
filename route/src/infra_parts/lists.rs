use crate::prelude::*;

pub(crate) fn store_wallets(prefix: &str) -> Vec<String> {
    let Ok(keys) = petal::sdk::store_list(prefix, MAX_LIST_BYTES) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in keys {
        let rest = key.strip_prefix(prefix).unwrap_or(&key);
        if let Some(wallet) = rest.split('/').next()
            && is_safe_segment(wallet)
            && !out.iter().any(|existing| existing == wallet)
        {
            out.push(wallet.to_string());
        }
    }
    out.sort();
    out
}

pub(crate) fn vfs_wallets_or_store(store_prefix: &str) -> Vec<String> {
    match petal::sdk::vfs_list("wallets", MAX_LIST_BYTES) {
        Ok(names) => safe_wallet_names(names),
        Err(_) if store_prefix.is_empty() => Vec::new(),
        Err(_) => store_wallets(store_prefix),
    }
}

pub(crate) fn safe_wallet_names(names: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for name in names {
        if name != "new" && is_safe_segment(&name) && !out.iter().any(|existing| existing == &name)
        {
            out.push(name);
        }
    }
    out.sort();
    out
}

pub(crate) fn store_ids(prefix: &str, suffix: &str) -> Vec<String> {
    let Ok(keys) = petal::sdk::store_list(prefix, MAX_LIST_BYTES) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in keys {
        if let Some(rest) = key.strip_prefix(prefix)
            && let Some(id) = rest.strip_suffix(suffix)
            && !id.contains('/')
            && is_safe_segment(id)
            && !out.iter().any(|existing| existing == id)
        {
            out.push(id.to_string());
        }
    }
    out.sort();
    out
}

pub(crate) fn next_id(prefix: &str, suffix: &str) -> String {
    let next = store_ids(prefix, suffix)
        .into_iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    format!("{next:04}")
}
