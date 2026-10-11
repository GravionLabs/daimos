import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import type { Item } from "./bindings";
import { type Host, tauriHost } from "./host";
import { ResultList } from "./ResultList";
import {
  applyBatch,
  emptyResults,
  move,
  primaryAction,
  type Results,
  SESSION,
  scrollInto,
  selectAt,
  selectedIndex,
  VIEWPORT_ROWS,
} from "./results";
import { SearchField } from "./SearchField";

/** How often the component tree was created. The window is hidden and shown, never rebuilt. */
export const mounts = { count: 0 };

export function App(props: { host?: Host }) {
  const host = () => props.host ?? tauriHost;
  const [query, setQueryText] = createSignal("");
  const [results, setResults] = createSignal<Results>(emptyResults);
  const [first, setFirst] = createSignal(0);

  /** Sets the text and asks the core for results; the list keeps showing the old ones until the first batch. */
  const setQuery = (text: string) => {
    setQueryText(text);
    host().query(SESSION, text);
  };

  const select = (next: Results) => {
    setResults(next);
    setFirst(scrollInto(first(), selectedIndex(next), next.items.length));
  };

  const run = (item: Item | undefined) => {
    const action = item && primaryAction(item);
    if (item && action) host().runAction(item.id, action.id);
  };

  onMount(() => {
    mounts.count += 1;
    const unsubscribe = host().onShown(() => {
      setQuery("");
      // Two frames: the first applies the cleared state, the second is on screen.
      requestAnimationFrame(() => requestAnimationFrame(() => host().firstPaint()));
    });
    const unsubscribeQuery = host().onQuery(setQuery);
    const unsubscribeResults = host().onResults((batch) => {
      if (batch.session !== SESSION) return;
      const merged = applyBatch(results(), batch);
      // A new query starts at the top; a merge scrolls only if the selection left the viewport.
      if (batch.seq === 0) setFirst(0);
      select(merged);
    });
    onCleanup(() => {
      unsubscribe();
      unsubscribeQuery();
      unsubscribeResults();
    });
  });

  // The window is sized for the rows the list shows.
  createEffect(() => host().resize(Math.min(results().items.length, VIEWPORT_ROWS)));

  const onKeyDown = (event: KeyboardEvent) => {
    const current = results();
    const index = selectedIndex(current);
    const pick = (next: Results) => {
      event.preventDefault();
      select(next);
    };
    if (event.key === "ArrowDown") pick(move(current, 1));
    else if (event.key === "ArrowUp") pick(move(current, -1));
    else if (event.key === "Home") pick(selectAt(current, 0));
    else if (event.key === "End") pick(selectAt(current, current.items.length - 1));
    else if (event.key === "Enter") {
      event.preventDefault();
      run(current.items[index]);
    } else if ((event.ctrlKey || event.metaKey) && /^[1-9]$/.test(event.key)) {
      // Ctrl/Cmd+1…9 run the n-th visible result.
      event.preventDefault();
      run(current.items[first() + Number(event.key) - 1]);
    }
  };

  return (
    <main class="palette" onKeyDown={onKeyDown}>
      <SearchField
        query={query()}
        onQuery={setQuery}
        onEscape={() => {
          if (query() === "") host().hide();
          else setQuery("");
        }}
      />
      <ResultList
        items={results().items}
        selectedId={results().selected}
        first={first()}
        iconSrc={(icon) => host().iconSrc(icon)}
        onScroll={setFirst}
        onSelect={(index) => setResults(selectAt(results(), index))}
        onRun={(index) => run(results().items[index])}
      />
    </main>
  );
}
