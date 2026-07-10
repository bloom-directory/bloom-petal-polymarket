petal::route_file!(spec: petal::static_dir_spec(), list:
    petal::files(&[
        "portfolio.json",
        "orders.json",
        "status.json",
        "buying_power.json",
        "funding_options.json",
    ])
);
