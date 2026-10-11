//! Property tests for the ranking rules that must hold for any input.

use daimos_core::Item;
use daimos_core::frecency::Frecency;
use daimos_core::ranker::{Candidate, Ranker};
use proptest::prelude::*;

fn titles() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec("[a-zA-Z0-9 ]{1,16}", 1..40)
}

fn ranked_titles(query: &str, items: &[Item], f: &Frecency) -> Vec<(usize, bool)> {
    let cands: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
    Ranker::new()
        .rank(query, &cands, f, 0, usize::MAX)
        .into_iter()
        .map(|r| (r.index, r.prefix))
        .collect()
}

proptest! {
    #[test]
    fn the_same_input_gives_the_same_order(titles in titles(), query in "[a-z]{1,4}") {
        let items: Vec<_> = titles.iter().enumerate().map(|(i, t)| Item::new(format!("id{i}"), t.clone())).collect();
        let f = Frecency::in_memory().unwrap();
        prop_assert_eq!(ranked_titles(&query, &items, &f), ranked_titles(&query, &items, &f));
    }

    #[test]
    fn the_order_does_not_depend_on_the_input_order(titles in titles(), query in "[a-z]{1,4}") {
        let items: Vec<_> = titles.iter().enumerate().map(|(i, t)| Item::new(format!("id{i}"), t.clone())).collect();
        let mut reversed = items.clone();
        reversed.reverse();
        let f = Frecency::in_memory().unwrap();
        let ids = |items: &[Item]| ranked_titles(&query, items, &f).into_iter().map(|(i, _)| items[i].id.clone()).collect::<Vec<_>>();
        prop_assert_eq!(ids(&items), ids(&reversed));
    }

    #[test]
    fn prefix_matches_come_before_fuzzy_matches(titles in titles(), query in "[a-z]{1,3}", launches in 0usize..6) {
        let items: Vec<_> = titles.iter().enumerate().map(|(i, t)| Item::new(format!("id{i}"), t.clone())).collect();
        let f = Frecency::in_memory().unwrap();
        // Frecency must not lift a fuzzy match over a prefix match.
        for _ in 0..launches {
            f.record("id0", 0).unwrap();
        }
        let order = ranked_titles(&query, &items, &f);
        let first_fuzzy = order.iter().position(|&(_, prefix)| !prefix);
        if let Some(pos) = first_fuzzy {
            prop_assert!(order[pos..].iter().all(|&(_, prefix)| !prefix));
        }
        for &(i, prefix) in &order {
            prop_assert_eq!(prefix, items[i].title.to_lowercase().starts_with(&query));
        }
    }
}
