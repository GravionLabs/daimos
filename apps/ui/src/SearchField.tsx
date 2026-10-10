import { onMount } from "solid-js";

/** The search box. It owns nothing: the palette decides what the query is. */
export function SearchField(props: {
  query: string;
  onQuery: (query: string) => void;
  onEscape: () => void;
}) {
  let input: HTMLInputElement | undefined;
  // The window keeps its webview between shows, so the box grabs the focus again on every show.
  onMount(() => input?.focus());

  return (
    <input
      ref={input}
      class="search"
      type="search"
      placeholder="Search apps and commands"
      aria-label="Search"
      value={props.query}
      onInput={(event) => props.onQuery(event.currentTarget.value)}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          props.onEscape();
        }
      }}
    />
  );
}
