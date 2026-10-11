import { For } from "solid-js";
import type { Action } from "./bindings";

/** Height of one action row in CSS pixels. */
export const ACTION_ROW_HEIGHT = 40;

/** The actions of the selected item. The search field keeps the focus; the palette handles the keys. */
export function ActionPanel(props: {
  actions: Action[];
  selected: number;
  onSelect: (index: number) => void;
  onRun: (index: number) => void;
}) {
  return (
    <div class="actions" role="menu" aria-label="Actions">
      <For each={props.actions}>
        {(action, index) => (
          // biome-ignore lint/a11y/useKeyWithClickEvents: the keyboard works from the search field, which keeps the focus
          <div
            class="action"
            role="menuitem"
            tabIndex={-1}
            data-selected={index() === props.selected}
            onMouseMove={() => props.onSelect(index())}
            onClick={() => props.onRun(index())}
          >
            <span>{action.title}</span>
            {action.shortcut && <kbd>{action.shortcut}</kbd>}
          </div>
        )}
      </For>
    </div>
  );
}
