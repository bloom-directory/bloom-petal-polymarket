crate::bloom_dir_component!(crate::static_dir_spec(), {
    let mut out = vec![crate::writable("begin")];
    out.extend(crate::files(&crate::ONBOARD_FILES));
    out
});
