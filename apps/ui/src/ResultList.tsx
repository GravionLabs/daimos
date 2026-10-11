import { createEffect, createSignal, For, Show } from "solid-js";
import type { Icon, Item } from "./bindings";
import { highlightRuns, VIEWPORT_ROWS } from "./results";

/** Height of one row in CSS pixels; the window height in Rust (`RESULT_ROW_HEIGHT`) must match. */
export const ROW_HEIGHT = 52;

function Title(props: { title: string; highlight?: number[] }) {
  return (
    <span class="title">
      <For each={highlightRuns(props.title, props.highlight)}>
        {(run) => (run.hit ? <mark>{run.text}</mark> : run.text)}
      </For>
    </span>
  );
}

function ItemIcon(props: { icon: Icon | null; title: string; src: (icon: Icon) => string | null }) {
  const [broken, setBroken] = createSignal(false);
  const url = () => (props.icon ? props.src(props.icon) : null);
  return (
    <Show
      when={url() && !broken()}
      fallback={
        <span class="icon placeholder" aria-hidden="true">
          {Array.from(props.title)[0]?.toUpperCase()}
        </span>
      }
    >
      <img class="icon" src={url() ?? ""} alt="" onError={() => setBroken(true)} />
    </Show>
  );
}

/**
 * The result list. Only the visible rows (plus one on each side) are in the DOM, so a list of
 * thousands of items costs the same as one of ten.
 */
export function ResultList(props: {
  items: Item[];
  selectedId: string | null;
  /** Index of the first visible row. */
  first: number;
  iconSrc: (icon: Icon) => string | null;
  onScroll: (first: number) => void;
  onSelect: (index: number) => void;
  onRun: (index: number) => void;
}) {
  const start = () => Math.max(0, props.first - 1);
  const end = () => Math.min(props.items.length, props.first + VIEWPORT_ROWS + 1);
  const visible = () => props.items.slice(start(), end());
  let box: HTMLDivElement | undefined;

  // Keep the scroll position of the box in step with the first visible row, whatever moved it.
  createEffect(() => {
    const top = props.first * ROW_HEIGHT;
    if (box && Math.abs(box.scrollTop - top) >= 1) box.scrollTop = top;
  });

  return (
    <div
      ref={box}
      class="results"
      role="listbox"
      aria-label="Results"
      style={{ height: `${Math.min(props.items.length, VIEWPORT_ROWS) * ROW_HEIGHT}px` }}
      onScroll={(event) => {
        const first = Math.round(event.currentTarget.scrollTop / ROW_HEIGHT);
        if (first !== props.first) props.onScroll(first);
      }}
    >
      <div class="spacer" style={{ height: `${props.items.length * ROW_HEIGHT}px` }}>
        <For each={visible()}>
          {(item, offset) => {
            const index = () => start() + offset();
            return (
              // biome-ignore lint/a11y/useKeyWithClickEvents: the keyboard works from the search field, which keeps the focus (Enter runs the selected row)
              <div
                class="row"
                role="option"
                tabIndex={-1}
                aria-selected={item.id === props.selectedId}
                data-selected={item.id === props.selectedId}
                style={{ transform: `translateY(${index() * ROW_HEIGHT}px)` }}
                onMouseMove={() => item.id !== props.selectedId && props.onSelect(index())}
                onClick={() => props.onRun(index())}
              >
                <ItemIcon icon={item.icon} title={item.title} src={props.iconSrc} />
                <span class="text">
                  <Title title={item.title} highlight={item.highlight} />
                  <Show when={item.subtitle}>
                    <span class="subtitle">{item.subtitle}</span>
                  </Show>
                </span>
                <For each={item.accessories}>
                  {(accessory) => <span class="accessory">{accessory}</span>}
                </For>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}
