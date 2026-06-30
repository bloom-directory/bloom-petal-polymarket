crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = vec![crate::writable("begin")];
    out.extend(crate::files(&crate::ONBOARD_FILES));
    out
});
