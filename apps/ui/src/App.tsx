import { createSignal, onCleanup, onMount } from "solid-js";
import { type Host, tauriHost } from "./host";
import { SearchField } from "./SearchField";

/** How often the component tree was created. The window is hidden and shown, never rebuilt. */
export const mounts = { count: 0 };

export function App(props: { host?: Host }) {
  const host = () => props.host ?? tauriHost;
  const [query, setQuery] = createSignal("");

  onMount(() => {
    mounts.count += 1;
    const unsubscribe = host().onShown(() => {
      setQuery("");
      // Two frames: the first applies the cleared state, the second is on screen.
      requestAnimationFrame(() => requestAnimationFrame(() => host().firstPaint()));
    });
    const unsubscribeQuery = host().onQuery(setQuery);
    onCleanup(() => {
      unsubscribe();
      unsubscribeQuery();
    });
  });

  return (
    <main class="palette">
      <SearchField
        query={query()}
        onQuery={setQuery}
        onEscape={() => {
          if (query() === "") host().hide();
          else setQuery("");
        }}
      />
    </main>
  );
}
