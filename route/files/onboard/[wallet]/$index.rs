crate::route_file!(spec: crate::static_dir_spec(), list: {
    let mut out = crate::files(&["status.json", "plan.md", "approvals.json"]);
    out.push(crate::writable("begin"));
    out
});
