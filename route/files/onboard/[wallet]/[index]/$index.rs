petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut out = petal::files(&[
        "status.json",
        "plan.md",
        "approvals.json",
        "review_intent.json",
        "approval.json",
    ]);
    out.push(petal::writable("begin"));
    out
});
