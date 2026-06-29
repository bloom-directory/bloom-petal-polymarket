crate::bloom_dir_component!("onboard/[wallet]/$list", {
    let mut out = vec!["begin".to_string()];
    out.extend(crate::strings(&crate::ONBOARD_FILES));
    out
});
