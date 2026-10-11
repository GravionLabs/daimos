import { createEffect, createSignal, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { ACTION_ROW_HEIGHT, ActionPanel } from "./ActionPanel";
import type { Action, Detail, FormField, Item, View } from "./bindings";
import { DetailPane } from "./DetailPane";
import { FormView } from "./FormView";
import { type Host, tauriHost } from "./host";
import { ResultList, ROW_HEIGHT } from "./ResultList";
import {
  applyBatch,
  emptyResults,
  matchesShortcut,
  move,
  primaryAction,
  providerOf,
  type Results,
  SESSION,
  scrollInto,
  selectAt,
  selectedIndex,
  VIEWPORT_ROWS,
} from "./results";
import { SearchField } from "./SearchField";
import { SettingsView } from "./SettingsView";
import { applyTheme } from "./theme";

/** How often the component tree was created. The window is hidden and shown, never rebuilt. */
export const mounts = { count: 0 };

/** One level of the navigation stack: a view that an action pushed. */
type Frame =
  | {
      kind: "list";
      title: string;
      provider: string;
      /** The view the list came from, to reload it after an action changed it. */
      viewId: string | null;
      results: Results;
      first: number;
    }
  | { kind: "detail"; title: string; detail: Detail }
  | { kind: "form"; title: string; provider: string; viewId: string; fields: FormField[] };

const TOAST_MS = 1600;

/** The payload of the action that opens the settings page (see `OPEN_SETTINGS` in the core). */
const OPEN_SETTINGS = "daimos:settings";

export function App(props: { host?: Host }) {
  const host = () => props.host ?? tauriHost;
  const [query, setQueryText] = createSignal("");
  const [root, setRoot] = createSignal<Results>(emptyResults);
  const [rootFirst, setRootFirst] = createSignal(0);
  const [stack, setStack] = createSignal<Frame[]>([]);
  const [panel, setPanel] = createSignal<{ selected: number } | null>(null);
  const [showDetail, setShowDetail] = createSignal(false);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [toast, setToast] = createSignal<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  const top = () => stack().at(-1);
  /** The list the keyboard works on: the pushed list view, or the search results. */
  const list = (): { results: Results; first: number } => {
    const frame = top();
    return frame?.kind === "list"
      ? { results: frame.results, first: frame.first }
      : { results: root(), first: rootFirst() };
  };
  const setList = (results: Results, first: number) => {
    const frame = top();
    if (frame?.kind === "list") {
      setStack([...stack().slice(0, -1), { ...frame, results, first }]);
    } else {
      setRoot(results);
      setRootFirst(first);
    }
  };

  const selectedItem = () => list().results.items[selectedIndex(list().results)];

  const notify = (message: string) => {
    setToast(message);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => setToast(null), TOAST_MS);
  };

  /** Sets the text and asks the core for results; the list keeps showing the old ones until the first batch. */
  const setQuery = (text: string) => {
    setQueryText(text);
    host().query(SESSION, text);
  };

  const select = (next: Results) => {
    const { first } = list();
    setList(next, scrollInto(first, selectedIndex(next), next.items.length));
  };

  const closeTransient = () => {
    setSettingsOpen(false);
    setPanel(null);
    setShowDetail(false);
    setStack([]);
  };

  const push = (frame: Frame) => {
    setPanel(null);
    setStack([...stack(), frame]);
  };

  const showView = (title: string, provider: string, viewId: string | null, view: View) => {
    if (view.type === "list") {
      const results = applyBatch(emptyResults, {
        session: SESSION,
        seq: 0,
        provider,
        items: view.items,
        done: true,
        weight: 1,
      });
      push({ kind: "list", title, provider, viewId, results, first: 0 });
    } else if (view.type === "detail") {
      push({ kind: "detail", title, detail: view.detail });
    } else if (view.type === "form") {
      push({ kind: "form", title, provider, viewId: viewId ?? "", fields: view.fields });
    } else {
      // A view that points to another view: follow it.
      void open(title, provider, view.viewId);
    }
  };

  const open = async (title: string, provider: string, viewId: string) => {
    try {
      showView(title, provider, viewId, await host().loadView(provider, viewId));
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
    }
  };

  /** Shows the list again as the core has it now: the search results, or the pushed view. */
  const refresh = async () => {
    const frame = top();
    if (frame?.kind !== "list") {
      host().query(SESSION, query());
      return;
    }
    if (!frame.viewId) return;
    const view = await host().loadView(frame.provider, frame.viewId);
    if (view.type !== "list") return;
    const fresh = applyBatch(emptyResults, {
      session: SESSION,
      seq: 0,
      provider: frame.provider,
      items: view.items,
      done: true,
      weight: 1,
    });
    // Keep the selection where it was, or on the last item when the list got shorter.
    const index = Math.min(selectedIndex(frame.results), fresh.items.length - 1);
    const next = selectAt(fresh, Math.max(0, index));
    setList(next, scrollInto(frame.first, selectedIndex(next), next.items.length));
  };

  /** Runs one action of an item: what happens depends on its kind. */
  const runItemAction = (item: Item, action: Action) => {
    const provider = providerOf(list().results, item.id) ?? "";
    setPanel(null);
    switch (action.kind) {
      case "copy":
        host()
          .copy(action.payload ?? item.title)
          .then(() => notify("Copied"))
          .catch(() => notify("Could not copy"));
        break;
      case "paste":
        // The web view puts the text on the clipboard (it works on every compositor), the shell
        // sends the shortcut. Where it cannot, the text stays copied and the window says why.
        host()
          .copy(action.payload ?? item.title)
          .then(() => host().paste())
          .then((result) => {
            if (!result.pasted) notify(`Copied. ${result.reason ?? "Paste it yourself."}`);
          })
          .catch(() => notify("Could not copy"));
        break;
      case "push":
      case "form":
        if (action.payload === OPEN_SETTINGS) setSettingsOpen(true);
        else if (action.payload) void open(action.title, provider, action.payload);
        break;
      default:
        // Whatever the action changed (a pin, a deletion) shows in the list afterwards.
        host()
          .runAction(item.id, action.id)
          .then(refresh)
          .catch((error) => notify(error instanceof Error ? error.message : String(error)));
    }
  };

  const runPrimary = (item: Item | undefined) => {
    const action = item && primaryAction(item);
    if (item && action) runItemAction(item, action);
  };

  const back = () => setStack(stack().slice(0, -1));

  /** Reads the theme and applies it; a failure keeps the CSS defaults (the OS scheme). */
  const refreshTheme = () => {
    host()
      .theme()
      .then((info) => applyTheme(info))
      .catch(() => {});
  };

  onMount(() => {
    mounts.count += 1;
    refreshTheme();
    const unsubscribeTheme = host().onTheme((info) => applyTheme(info));
    const unsubscribeSettings = host().onOpenSettings(() => setSettingsOpen(true));
    const unsubscribe = host().onShown(() => {
      // The accent can change while the window is hidden.
      refreshTheme();
      closeTransient();
      setQuery("");
      // Two frames: the first applies the cleared state, the second is on screen.
      requestAnimationFrame(() => requestAnimationFrame(() => host().firstPaint()));
    });
    const unsubscribeQuery = host().onQuery(setQuery);
    const unsubscribeResults = host().onResults((batch) => {
      if (batch.session !== SESSION) return;
      const merged = applyBatch(root(), batch);
      // A new query starts at the top; a merge scrolls only if the selection left the viewport.
      const first = batch.seq === 0 ? 0 : rootFirst();
      setRoot(merged);
      setRootFirst(scrollInto(first, selectedIndex(merged), merged.items.length));
    });
    // Every listener is set up: the shell replays a show or a text it sent before this point.
    void host().ready();
    onCleanup(() => {
      unsubscribeTheme();
      unsubscribeSettings();
      unsubscribe();
      unsubscribeQuery();
      unsubscribeResults();
      clearTimeout(toastTimer);
    });
  });

  const panelRows = () => {
    const open = panel();
    const item = selectedItem();
    return open && item
      ? Math.ceil((item.actions.length * ACTION_ROW_HEIGHT + 16) / ROW_HEIGHT)
      : 0;
  };

  // The window is sized for what it shows: the result rows, a roomier detail pane, the panel, a form.
  createEffect(() => {
    const frame = top();
    let rows: number;
    if (settingsOpen()) {
      host().resize(VIEWPORT_ROWS);
      return;
    }
    if (frame?.kind === "form") rows = frame.fields.length + 2;
    else if (frame?.kind === "detail") rows = VIEWPORT_ROWS;
    else rows = Math.min(list().results.items.length, VIEWPORT_ROWS);
    if (showDetail() && frame?.kind !== "form") rows = Math.max(rows, 5);
    rows = Math.max(rows, panelRows());
    host().resize(Math.min(rows, VIEWPORT_ROWS));
  });

  const onEscape = () => {
    if (settingsOpen()) setSettingsOpen(false);
    else if (panel()) setPanel(null);
    else if (stack().length > 0) back();
    else if (showDetail()) setShowDetail(false);
    else if (query() !== "") setQuery("");
    else host().hide();
  };

  const onKeyDown = (event: KeyboardEvent) => {
    // Ctrl/Cmd+, opens the settings from anywhere in the palette (and closes them again).
    if ((event.ctrlKey || event.metaKey) && event.key === ",") {
      event.preventDefault();
      setSettingsOpen(!settingsOpen());
      return;
    }
    // The settings page has its own keys; Esc closes it (the search field is gone then).
    if (settingsOpen()) {
      if (event.key === "Escape") {
        event.preventDefault();
        setSettingsOpen(false);
      }
      return;
    }
    // Pushed views have no search field, so Esc is handled here: close the panel, else go back.
    if (event.key === "Escape" && stack().length > 0) {
      event.preventDefault();
      onEscape();
      return;
    }
    // A form keeps its own keys; a detail view has no list.
    const frame = top();
    if (frame?.kind === "form" || frame?.kind === "detail") return;
    const { results, first } = list();
    const item = selectedItem();
    const open = panel();
    const pick = (next: Results) => {
      event.preventDefault();
      select(next);
    };

    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      setPanel(open ? null : item ? { selected: 0 } : null);
      return;
    }
    if (open && item) {
      const last = item.actions.length - 1;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setPanel({ selected: Math.min(last, open.selected + 1) });
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setPanel({ selected: Math.max(0, open.selected - 1) });
      } else if (event.key === "Enter") {
        event.preventDefault();
        const action = item.actions[open.selected];
        if (action) runItemAction(item, action);
      } else if (event.key === "Escape") {
        event.preventDefault();
        setPanel(null);
      }
      return;
    }
    // The shortcut an action names runs it (Ctrl+P pins a clipboard entry); Enter is the primary.
    const named = item?.actions.find(
      (action) =>
        action.shortcut && action.shortcut !== "Enter" && matchesShortcut(event, action.shortcut),
    );
    if (item && named) {
      event.preventDefault();
      runItemAction(item, named);
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      setShowDetail(!showDetail());
    } else if (event.key === "ArrowDown") pick(move(results, 1));
    else if (event.key === "ArrowUp") pick(move(results, -1));
    else if (event.key === "Home") pick(selectAt(results, 0));
    else if (event.key === "End") pick(selectAt(results, results.items.length - 1));
    else if (event.key === "Enter") {
      event.preventDefault();
      runPrimary(item);
    } else if ((event.ctrlKey || event.metaKey) && /^[1-9]$/.test(event.key)) {
      // Ctrl/Cmd+1…9 run the n-th visible result.
      event.preventDefault();
      runPrimary(results.items[first + Number(event.key) - 1]);
    }
  };

  const listView = () => (
    <div class="split" data-detail={showDetail()}>
      <ResultList
        items={list().results.items}
        selectedId={list().results.selected}
        first={list().first}
        iconSrc={(icon) => host().iconSrc(icon)}
        onScroll={(first) => setList(list().results, first)}
        onSelect={(index) => setList(selectAt(list().results, index), list().first)}
        onRun={(index) => runPrimary(list().results.items[index])}
      />
      <Show when={showDetail()}>
        <DetailPane detail={selectedItem()?.detail ?? null} />
      </Show>
    </div>
  );

  return (
    <main class="palette" onKeyDown={onKeyDown}>
      <Show when={settingsOpen()}>
        <SettingsView host={host()} onClose={() => setSettingsOpen(false)} />
      </Show>
      <Show when={!settingsOpen()}>
        <Show
          when={stack().length > 0}
          fallback={
            <SearchField
              query={query()}
              onQuery={setQuery}
              onEscape={onEscape}
              onSettings={() => setSettingsOpen(true)}
            />
          }
        >
          <header class="crumb">
            <button type="button" class="back" aria-label="Back" onClick={back}>
              ←
            </button>
            <span>{top()?.title}</span>
          </header>
        </Show>
        <Switch>
          <Match when={top()?.kind === "form" && (top() as Extract<Frame, { kind: "form" }>)}>
            {(form) => (
              <FormView
                title={form().title}
                fields={form().fields}
                onSubmit={(values) => {
                  host()
                    .submitForm(form().provider, form().viewId, values)
                    .then(() => {
                      notify("Done");
                      back();
                    })
                    .catch((error) =>
                      notify(error instanceof Error ? error.message : String(error)),
                    );
                }}
              />
            )}
          </Match>
          <Match when={top()?.kind === "detail" && (top() as Extract<Frame, { kind: "detail" }>)}>
            {(detail) => <DetailPane detail={detail().detail} />}
          </Match>
          <Match when={true}>{listView()}</Match>
        </Switch>
      </Show>
      <Show when={panel() && selectedItem()}>
        {(item) => (
          <ActionPanel
            actions={item().actions}
            selected={panel()?.selected ?? 0}
            onSelect={(index) => setPanel({ selected: index })}
            onRun={(index) => {
              const action = item().actions[index];
              if (action) runItemAction(item(), action);
            }}
          />
        )}
      </Show>
      <Show when={toast()}>
        <output class="toast">{toast()}</output>
      </Show>
    </main>
  );
}
