import { onMount } from "solid-js";
import { isMac } from "./hotkey";

/** The search box. It owns nothing: the palette decides what the query is. */
export function SearchField(props: {
  query: string;
  onQuery: (query: string) => void;
  onEscape: () => void;
  /** Opens the settings; the button is the visible way to them besides the tray and Ctrl+,. */
  onSettings: () => void;
}) {
  let input: HTMLInputElement | undefined;
  // The window keeps its webview between shows, so the box grabs the focus again on every show.
  onMount(() => input?.focus());

  return (
    <div class="search-row">
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
      <button
        type="button"
        class="settings-button"
        aria-label="Settings"
        title={`Settings (${isMac() ? "Cmd" : "Ctrl"}+,)`}
        tabindex="-1"
        onClick={props.onSettings}
      >
        ⚙
      </button>
    </div>
  );
}
