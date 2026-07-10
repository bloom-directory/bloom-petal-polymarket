petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut children = petal::files(&[
        "plan.md",
        "request.json",
        "status.json",
        "review_intent.json",
        "approval.json",
    ]);
    children.push(petal::writable("confirm"));
    children
});
