pub(crate) fn wallets() -> Vec<crate::RouteChild> {
    crate::dirs(super::vfs_wallets_or_store("onboard/"))
}

pub(crate) fn files() -> Vec<crate::RouteChild> {
    let mut out = crate::files(&super::ONBOARD_FILES);
    out.push(crate::writable("begin"));
    out
}

pub(crate) fn begin_hint() -> crate::DispatchResponse {
    super::onboard_begin_hint()
}

pub(crate) fn begin_read(_wallet: &str) -> crate::DispatchResponse {
    super::onboard_begin_hint()
}

pub(crate) fn begin_write(wallet: &str) -> crate::DispatchResponse {
    super::write_onboard_begin(wallet)
}

pub(crate) fn status_json(wallet: &str) -> crate::DispatchResponse {
    super::onboard_status_json(wallet)
}

pub(crate) fn plan_md(wallet: &str) -> crate::DispatchResponse {
    super::onboard_plan_md(wallet)
}

pub(crate) fn approvals_json(wallet: &str) -> crate::DispatchResponse {
    super::onboard_approvals_json(wallet)
}
