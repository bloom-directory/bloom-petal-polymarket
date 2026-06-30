petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut out = petal::files(&["receipt.json"]);
    out.push(petal::writable("cancel"));
    out
});
