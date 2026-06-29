crate::bloom_dir_component!({
    let mut out = vec![crate::writable("begin")];
    out.extend(crate::files(&crate::ONBOARD_FILES));
    out
});
