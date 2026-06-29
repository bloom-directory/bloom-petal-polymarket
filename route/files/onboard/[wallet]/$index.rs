crate::bloom_dir_component!("onboard/[wallet]/$index", {
    let mut out = vec!["begin".to_string()];
    out.extend(crate::strings(&crate::ONBOARD_FILES));
    out
});
