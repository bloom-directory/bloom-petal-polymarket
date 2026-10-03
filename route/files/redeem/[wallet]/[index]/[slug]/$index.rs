petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut children = petal::files(&["plan.md", "review_intent.json", "approval.json", "receipt.json"]);
    children.push(petal::writable("confirm"));
    children
});
