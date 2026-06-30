petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut out = petal::files(&["status.json", "plan.md", "approvals.json"]);
    out.push(petal::writable("begin"));
    out
});
