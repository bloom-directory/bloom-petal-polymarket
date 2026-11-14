petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::wallet_param(ctx) { Ok(value) => value, Err(resp) => return resp };
    let account = match petal::wallet_account(ctx) { Ok(value) => value, Err(resp) => return resp };
    use crate::prelude::*;
    use crate::polymarket::validate_wallet_name;
    use crate::polymarket::eip712::USDC_E;

    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    let fund_route = format!("fund/{wallet}/{}/new", account.number());
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "target_asset": "pUSD",
        "options": [{
            "from": "pUSD",
            "supported": !legacy_eoa,
            "review_required": true,
            "fund_route": fund_route,
            "execution": "generic_evm_outbox_direct_erc20_transfer",
        }, {
            "from": "USDC.e",
            "token": USDC_E.to_checksum(None),
            "supported": !legacy_eoa,
            "review_required": true,
            "fund_route": fund_route,
            "execution": "polymarket_collateral_onramp_wrap",
        }, {
            "from": "other",
            "supported": false,
            "next": format!("swap to USDC.e ({}) on Polygon, for example with the Enso Petal, then fund from USDC.e", USDC_E.to_checksum(None)),
        }],
        "credentials_read_only": legacy_eoa,
    }))

});
