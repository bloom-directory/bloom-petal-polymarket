petal::route_file!(spec: petal::static_dir_spec(), list:
    petal::files(&["positions.json", "trades.json", "activity.json"])
);
