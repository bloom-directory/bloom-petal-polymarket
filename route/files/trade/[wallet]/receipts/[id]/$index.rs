crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = crate::files(&["receipt.json"]);
    out.push(crate::writable("cancel"));
    out
});
