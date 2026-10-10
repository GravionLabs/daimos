import { createSignal } from "solid-js";

/** Placeholder search field: it holds the query and nothing else yet. */
export function SearchField() {
  const [query, setQuery] = createSignal("");
  return (
    <input
      class="search"
      type="search"
      placeholder="Search apps and commands"
      aria-label="Search"
      autofocus
      value={query()}
      onInput={(event) => setQuery(event.currentTarget.value)}
    />
  );
}
