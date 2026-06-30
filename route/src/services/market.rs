pub(crate) fn files() -> Vec<crate::RouteChild> {
    crate::files(&super::MARKET_FILES)
}

pub(crate) fn slugs() -> Result<Vec<String>, crate::DispatchResponse> {
    super::list_market_slugs()
}

pub(crate) fn market_json(slug: &str) -> crate::DispatchResponse {
    super::market_json(slug)
}

pub(crate) fn book_json(slug: &str) -> crate::DispatchResponse {
    super::market_book_json(slug)
}

pub(crate) fn prices_json(slug: &str) -> crate::DispatchResponse {
    super::market_prices_json(slug)
}
