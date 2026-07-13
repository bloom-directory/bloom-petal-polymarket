petal::route_file!(spec: petal::chain_read_spec().caps(&["bloom:store", "bloom:vfs.read"]), read: |ctx: &petal::Ctx| {
    let wallet = match petal::param(ctx, "wallet") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    if let Err(e) = crate::polymarket::validate_wallet_name(wallet) {
        return petal::error(-3, e.to_string());
    }
    let (_, chain_id) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return petal::error(-4, err),
    };
    let owner = match crate::infra_parts::host_calls::wallet_address(wallet) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let status = crate::onboarding::stored_status_for_wallet(wallet, owner).ok();
    let deposit = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("address"))
                .and_then(serde_json::Value::as_str)
        })
        .and_then(|address| address.parse::<alloy::primitives::Address>().ok())
        .unwrap_or_else(|| crate::polymarket::eip712::derive_deposit_wallet_address(&owner, chain_id));
    let deposit_source = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("source"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("local_estimate_unverified");
    let raw_deposit_fundable = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("fundable"))
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false);
    let deposit_fundable = raw_deposit_fundable && deposit_source == "live_factory_resolved";
    let calls: Vec<serde_json::Value> = crate::polymarket::wallet::v2_approval_calls()
        .iter()
        .zip(crate::polymarket::wallet::V2_APPROVAL_LABELS)
        .map(|(call, label)| {
            serde_json::json!({
                "label": label,
                "target": format!("{:#x}", call.target),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(call.data.as_ref())),
            })
        })
        .collect();
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "owner": format!("{owner:#x}"),
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": deposit_source,
        "deposit_wallet_fundable": deposit_fundable,
        "warning": if deposit_fundable {
            serde_json::Value::Null
        } else {
            serde_json::Value::String("do not fund this locally derived estimate; full onboarding must resolve the live factory address before funding or approvals".into())
        },
        "chain_id": chain_id,
        "calls": calls,
        "signing": "preview_only"
    }))
});
