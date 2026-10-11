//! Matching and ranking: `nucleo-matcher` scores, frecency and provider weight blend.
//!
//! `rank = match score × provider weight + frecency boost`. An exact prefix match of the title
//! always beats a fuzzy match, so the ranking is a tier first (prefix, then fuzzy) and the blended
//! value second. Ties break on title and id, so the same input always gives the same order.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::frecency::Frecency;
use crate::model::Item;

/// Each unit of frecency (one launch today) is worth this much match score. Three launches
/// outweigh the difference between two prefix matches of two letters.
pub const FRECENCY_WEIGHT: f64 = 60.0;

/// Keyword matches count less than title matches.
const KEYWORD_FACTOR: f64 = 0.7;

/// An item to rank with the weight of the provider it came from.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub item: &'a Item,
    pub weight: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub index: usize,
    pub score: f64,
    pub prefix: bool,
    /// Character positions in the title for highlighting; empty when only a keyword matched.
    pub title_indices: Vec<u32>,
}

pub struct Ranker {
    matcher: Matcher,
    buf: Vec<char>,
    indices: Vec<u32>,
}

impl Default for Ranker {
    fn default() -> Self {
        Self::new()
    }
}

impl Ranker {
    pub fn new() -> Self {
        Self { matcher: Matcher::new(Config::DEFAULT), buf: Vec::new(), indices: Vec::new() }
    }

    /// Ranks `candidates` for `query` at `now`; the result holds the indices into `candidates`,
    /// best first. An empty query lists the most frecent items only.
    pub fn rank(
        &mut self,
        query: &str,
        candidates: &[Candidate<'_>],
        frecency: &Frecency,
        now: i64,
        limit: usize,
    ) -> Vec<Ranked> {
        let query = query.trim();
        let mut ranked = if query.is_empty() {
            self.recent(candidates, frecency, now)
        } else {
            self.matching(query, candidates, frecency, now)
        };
        ranked.sort_by(|a, b| {
            b.prefix
                .cmp(&a.prefix)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| candidates[a.index].item.title.cmp(&candidates[b.index].item.title))
                .then_with(|| candidates[a.index].item.id.cmp(&candidates[b.index].item.id))
        });
        ranked.truncate(limit);
        ranked
    }

    fn recent(&self, candidates: &[Candidate<'_>], frecency: &Frecency, now: i64) -> Vec<Ranked> {
        candidates
            .iter()
            .enumerate()
            .filter_map(|(index, c)| {
                let boost = frecency.score(&c.item.id, now);
                (boost > 0.0).then(|| Ranked {
                    index,
                    score: boost,
                    prefix: false,
                    title_indices: Vec::new(),
                })
            })
            .collect()
    }

    fn matching(
        &mut self,
        query: &str,
        candidates: &[Candidate<'_>],
        frecency: &Frecency,
        now: i64,
    ) -> Vec<Ranked> {
        let pattern =
            Pattern::new(query, CaseMatching::Smart, Normalization::Smart, AtomKind::Fuzzy);
        let lowered = query.to_lowercase();
        let mut out = Vec::new();
        for (index, c) in candidates.iter().enumerate() {
            self.indices.clear();
            let title = pattern.indices(
                Utf32Str::new(&c.item.title, &mut self.buf),
                &mut self.matcher,
                &mut self.indices,
            );
            let title_indices =
                if title.is_some() { std::mem::take(&mut self.indices) } else { Vec::new() };
            let mut best = title.map(f64::from);
            for keyword in &c.item.keywords {
                let score = pattern.score(Utf32Str::new(keyword, &mut self.buf), &mut self.matcher);
                if let Some(score) = score {
                    let score = f64::from(score) * KEYWORD_FACTOR;
                    best = Some(best.map_or(score, |b| b.max(score)));
                }
            }
            let Some(matched) = best else { continue };
            let prefix = c.item.title.to_lowercase().starts_with(&lowered);
            let boost = frecency.score(&c.item.id, now) * FRECENCY_WEIGHT;
            out.push(Ranked {
                index,
                score: matched * f64::from(c.weight) + boost,
                prefix,
                title_indices,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(titles: &[&str]) -> Vec<Item> {
        titles.iter().map(|t| Item::new(*t, *t)).collect()
    }

    fn rank(ranker: &mut Ranker, q: &str, items: &[Item], f: &Frecency) -> Vec<String> {
        let cands: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
        ranker.rank(q, &cands, f, 0, 50).into_iter().map(|r| items[r.index].title.clone()).collect()
    }

    #[test]
    fn a_prefix_beats_a_better_scoring_fuzzy_match() {
        let f = Frecency::in_memory().unwrap();
        let items = items(&["Terminal Foo", "Firefox", "Fire Fox Browser", "F i r e"]);
        let order = rank(&mut Ranker::new(), "fire", &items, &f);
        assert!(order[..2].contains(&"Firefox".to_owned()));
        assert!(order[..2].contains(&"Fire Fox Browser".to_owned()));
    }

    #[test]
    fn non_matching_items_are_dropped_and_indices_point_into_the_title() {
        let f = Frecency::in_memory().unwrap();
        let items = items(&["Firefox", "Calculator"]);
        let cands: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
        let ranked = Ranker::new().rank("ffx", &cands, &f, 0, 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].title_indices, [0, 4, 6]);
    }

    #[test]
    fn keywords_match_when_the_title_does_not() {
        let f = Frecency::in_memory().unwrap();
        let mut item = Item::new("ff", "Firefox");
        item.keywords = vec!["browser".into()];
        let cands = [Candidate { item: &item, weight: 1.0 }];
        let ranked = Ranker::new().rank("brow", &cands, &f, 0, 10);
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].title_indices.is_empty());
    }

    #[test]
    fn an_item_chosen_three_times_ranks_first_for_its_first_two_letters() {
        let f = Frecency::in_memory().unwrap();
        let items = items(&["Files", "Firefox", "Fitness"]);
        for _ in 0..3 {
            f.record("Firefox", 0).unwrap();
        }
        assert_eq!(rank(&mut Ranker::new(), "fi", &items, &f)[0], "Firefox");
    }

    #[test]
    fn provider_weight_scales_the_match() {
        let f = Frecency::in_memory().unwrap();
        let a = Item::new("a", "Notes");
        let b = Item::new("b", "Notes");
        let cands = [Candidate { item: &a, weight: 1.0 }, Candidate { item: &b, weight: 2.0 }];
        let ranked = Ranker::new().rank("notes", &cands, &f, 0, 10);
        assert_eq!(ranked[0].index, 1);
    }

    #[test]
    fn the_empty_query_lists_the_most_frecent_items() {
        let f = Frecency::in_memory().unwrap();
        let items = items(&["A", "B", "C"]);
        f.record("C", 0).unwrap();
        f.record("C", 0).unwrap();
        f.record("A", 0).unwrap();
        assert_eq!(rank(&mut Ranker::new(), "", &items, &f), ["C", "A"]);
    }

    #[test]
    fn the_limit_applies() {
        let f = Frecency::in_memory().unwrap();
        let items = items(&["aa", "ab", "ac"]);
        let cands: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
        assert_eq!(Ranker::new().rank("a", &cands, &f, 0, 2).len(), 2);
    }
}
