//! The same script gives the same items whether it runs once per query or as a long-running
//! process. The tests skip when there is no Python.

mod common;

use common::Installed;
use daimos_core::Item;
use daimos_core::model::ItemBatch;
use daimos_ext::Backend;

fn items(batches: Vec<ItemBatch>) -> Vec<Item> {
    batches.into_iter().flat_map(|batch| batch.items).collect()
}

#[tokio::test]
async fn both_modes_return_identical_items_for_the_same_queries() {
    let python = python_or_skip!();
    let one_shot = Installed::new(python, false);
    let long_running = Installed::new(python, true);
    for text in ["conf", "conf hello", "conf two words", "conf ünï \"quoted\"", "conf "] {
        let expected = items(one_shot.batches(text).await);
        assert_eq!(expected.len(), 3, "{text:?}");
        let actual = items(long_running.batches(text).await);
        assert_eq!(actual, expected, "{text:?}");
    }
}

#[tokio::test]
async fn the_items_carry_ids_titles_subtitles_and_actions_in_both_modes() {
    let python = python_or_skip!();
    for long_running in [false, true] {
        let ext = Installed::new(python, long_running);
        let got = items(ext.batches("conf x").await);
        let ids: Vec<_> = got.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            ["conf one conf x", "conf two conf x", "conf three conf x"],
            "long_running={long_running}"
        );
        assert_eq!(got[0].subtitle.as_deref(), Some("from echo.py"));
        assert_eq!(got[0].actions.len(), 2);
        assert_eq!(got[0].actions[0].id, "open");
        assert_eq!(got[0].actions[1].id, "star");
    }
}

#[test]
fn a_custom_action_gives_the_same_toast_in_both_modes() {
    let python = python_or_skip!();
    let toasts: Vec<_> = [false, true]
        .into_iter()
        .map(|mode| {
            Installed::new(python, mode).backend.custom_action("echo", "search", "i1", "star")
        })
        .collect();
    assert_eq!(toasts[0], Ok(Some("Starred i1".to_owned())));
    assert_eq!(toasts[1], toasts[0]);
}
