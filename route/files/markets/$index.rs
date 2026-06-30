crate::route_file!(spec: crate::http_dir_spec(), fallible_list:
    {
        let url = crate::url_with_query(
            &format!("{}{}", crate::GAMMA, "/markets"),
            &[
                ("closed", "false"),
                ("limit", &crate::MARKETS_LIST_LIMIT.to_string()),
                ("order", "volumeNum"),
                ("ascending", "false"),
            ],
        );
        match crate::get_json::<Vec<crate::polymarket::Market>>(&url) {
            Ok(markets) => Ok(crate::dirs(
                markets
                    .into_iter()
                    .filter_map(|market| (!market.slug.is_empty()).then_some(market.slug))
                    .collect(),
            )),
            Err(resp) => Err(resp),
        }
    }
);
