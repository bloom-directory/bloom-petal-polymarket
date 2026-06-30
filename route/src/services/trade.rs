pub(crate) fn wallets() -> Vec<crate::RouteChild> {
    crate::dirs(super::vfs_wallets_or_store("trade/"))
}

pub(crate) fn draft_ids(wallet: &str) -> Result<Vec<crate::RouteChild>, crate::DispatchResponse> {
    Ok(crate::dirs(super::store_ids(
        &format!("trade/{wallet}/drafts/"),
        "/order.json",
    )))
}

pub(crate) fn receipt_ids(wallet: &str) -> Result<Vec<crate::RouteChild>, crate::DispatchResponse> {
    Ok(crate::dirs(super::store_ids(
        &format!("trade/{wallet}/receipts/"),
        "/receipt.json",
    )))
}

pub(crate) fn draft_files() -> Vec<crate::RouteChild> {
    let mut out = crate::files(&super::DRAFT_FILES);
    out.extend(
        super::DRAFT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
}

pub(crate) fn receipt_files() -> Vec<crate::RouteChild> {
    let mut out = crate::files(&super::RECEIPT_FILES);
    out.extend(
        super::RECEIPT_WRITABLE_FILES
            .iter()
            .map(|name| crate::writable(*name)),
    );
    out
}

pub(crate) fn new_hint() -> crate::DispatchResponse {
    crate::DispatchResponse::Read(super::TRADE_NEW_HINT.into())
}

pub(crate) fn create(wallet: &str, body: &[u8]) -> crate::DispatchResponse {
    super::write_trade_new(wallet, body)
}

pub(crate) fn revalidate_hint(wallet: &str, id: &str) -> crate::DispatchResponse {
    let _ = (wallet, id);
    super::trade_revalidate_hint()
}

pub(crate) fn revalidate(wallet: &str, id: &str, body: &[u8]) -> crate::DispatchResponse {
    super::write_trade_revalidate(wallet, id, body)
}

pub(crate) fn post_hint(wallet: &str, id: &str) -> crate::DispatchResponse {
    let _ = (wallet, id);
    super::trade_post_hint()
}

pub(crate) fn post(wallet: &str, id: &str, body: &[u8]) -> crate::DispatchResponse {
    super::write_trade_post(wallet, id, body)
}

pub(crate) fn cancel_hint(wallet: &str, id: &str) -> crate::DispatchResponse {
    let _ = (wallet, id);
    super::trade_cancel_hint()
}

pub(crate) fn cancel(wallet: &str, id: &str, body: &[u8]) -> crate::DispatchResponse {
    super::write_trade_cancel(wallet, id, body)
}
