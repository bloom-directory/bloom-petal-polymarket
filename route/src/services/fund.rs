pub(crate) fn wallets() -> Vec<crate::RouteChild> {
    crate::dirs(super::vfs_wallets_or_store("fund/"))
}

pub(crate) fn session_ids(wallet: &str) -> Vec<crate::RouteChild> {
    crate::dirs(super::store_ids(
        &format!("fund/{wallet}/requests/"),
        ".json",
    ))
}

pub(crate) fn files() -> Vec<crate::RouteChild> {
    crate::files(&super::FUND_FILES)
}

pub(crate) fn new_hint() -> crate::DispatchResponse {
    crate::DispatchResponse::Read(super::FUND_NEW_HINT.into())
}

pub(crate) fn create(wallet: &str, body: &[u8]) -> crate::DispatchResponse {
    super::write_fund_new(wallet, body)
}
