petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
    let account = match crate::account::number(ctx) { Ok(value) => value, Err(resp) => return resp };
    use crate::prelude::*;
    use crate::polymarket::validate_wallet_name;
    use crate::account_views::{load_enso_api_key, load_enso_router};

    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "target_asset": "pUSD",
        "options": [{
            "from": "pUSD",
            "supported": !legacy_eoa,
            "review_required": true,
            "fund_route": format!("fund/{wallet}/{account}/new"),
            "execution": "generic_evm_outbox_direct_erc20_transfer",
        }, {
            "from": "native_or_other_erc20",
            "supported": !legacy_eoa,
            "review_required": true,
            "fund_route": format!("fund/{wallet}/{account}/new"),
            "execution": "enso_quote_then_generic_evm_outbox",
            "enso_key_configured": load_enso_api_key().is_ok(),
            "enso_router_configured": load_enso_router().is_ok(),
        }],
        "credentials_read_only": legacy_eoa,
    }))

});
