import { createSignal, For, onMount, Show } from "solid-js";
import type {
  CapabilityList_Serialize as CapabilityList,
  ExtensionInfo_Serialize as Extension,
  PreferenceInfo_Serialize as Preference,
} from "./bindings";
import type { Host } from "./host";

const CLIPBOARD_LABEL: Record<string, string> = {
  read: "read",
  write: "write",
  "read-write": "read and write",
};

const STATE_LABEL: Record<string, string> = {
  enabled: "Enabled",
  "needs-confirmation": "Needs confirmation",
  error: "Error",
};

/** One line per capability; `isNew` marks what an update asks for beyond what was confirmed. */
export function capabilityLines(
  all: CapabilityList,
  added: CapabilityList | null,
): { text: string; isNew: boolean }[] {
  const lines: { text: string; isNew: boolean }[] = [];
  for (const fs of all.filesystem) {
    const isNew = added?.filesystem.some((a) => a.path === fs.path && a.write === fs.write);
    lines.push({
      text: `Files: ${fs.path} (${fs.write ? "read and write" : "read only"})`,
      isNew: isNew ?? false,
    });
  }
  for (const host of all.network) {
    lines.push({ text: `Network: ${host}`, isNew: added?.network.includes(host) ?? false });
  }
  if (all.clipboard) {
    lines.push({
      text: `Clipboard: ${CLIPBOARD_LABEL[all.clipboard] ?? all.clipboard}`,
      isNew: added?.clipboard != null,
    });
  }
  for (const entry of all.keyring) {
    lines.push({ text: `Keyring: ${entry}`, isNew: added?.keyring.includes(entry) ?? false });
  }
  return lines;
}

/** The extensions: their runtime label, the capability confirmation and their preferences. */
export function ExtensionsSection(props: { host: Host }) {
  const [extensions, setExtensions] = createSignal<Extension[] | null>(null);
  const [problem, setProblem] = createSignal<string | null>(null);

  const load = async () => {
    try {
      setExtensions((await props.host.extensions()).extensions);
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    }
  };
  onMount(() => void load());

  /** Runs a change, then shows what the core stored; a failure stays visible. */
  const change = async (action: () => Promise<void>) => {
    try {
      await action();
      setProblem(null);
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    }
    await load();
  };

  return (
    <>
      <h3>Extensions</h3>
      <Show when={problem()}>
        <p class="error" role="alert">
          {problem()}
        </p>
      </Show>
      <Show when={extensions() && extensions()?.length === 0}>
        <p class="hint">No extensions installed.</p>
      </Show>
      <ul class="extensions">
        <For each={extensions() ?? []}>
          {(extension) => (
            <ExtensionRow
              extension={extension}
              onConfirm={() => change(() => props.host.confirmExtension(extension.name))}
              onSet={(key, value) =>
                change(() => props.host.setPreference(extension.name, key, value))
              }
              onClear={(key) => change(() => props.host.clearPreference(extension.name, key))}
            />
          )}
        </For>
      </ul>
    </>
  );
}

function ExtensionRow(props: {
  extension: Extension;
  onConfirm: () => void;
  onSet: (key: string, value: string) => Promise<void>;
  onClear: (key: string) => Promise<void>;
}) {
  const extension = () => props.extension;
  const pending = () => extension().state === "needs-confirmation";
  // The first confirmation has nothing to compare with: everything is asked, nothing is "new".
  const marked = () => (extension().confirmed ? (extension().added ?? null) : null);
  const lines = () => capabilityLines(extension().capabilities, marked());
  return (
    <li class="extension" data-state={extension().state}>
      <div class="extension-head">
        <strong>{extension().title ?? extension().name}</strong>
        <Show when={extension().version}>
          <span class="version">{extension().version}</span>
        </Show>
        <span class="state">{STATE_LABEL[extension().state] ?? extension().state}</span>
        <Show when={extension().runtime}>
          <span class="badge" data-sandboxed={extension().sandboxed}>
            {extension().sandboxed ? "Sandboxed" : "Runs unsandboxed"}
          </span>
        </Show>
      </div>
      <Show when={extension().message}>
        <p class="hint">{extension().message}</p>
      </Show>
      <Show when={extension().state !== "error"}>
        <Show when={lines().length > 0} fallback={<p class="hint">It declares no capabilities.</p>}>
          <ul class="capability-list" aria-label={`Capabilities of ${extension().name}`}>
            <For each={lines()}>
              {(line) => (
                <li data-new={line.isNew}>
                  {line.text}
                  <Show when={line.isNew}>
                    <span class="new-tag"> (new)</span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
      <Show when={pending()}>
        <p class="hint">
          {extension().confirmed
            ? "A new version asks for more than you confirmed. It stays off until you confirm."
            : "It stays off until you confirm these capabilities."}
        </p>
        <button
          type="button"
          class="confirm"
          aria-label={`Confirm ${extension().name}`}
          onClick={props.onConfirm}
        >
          Confirm
        </button>
      </Show>
      <Show when={extension().preferences.length > 0}>
        <fieldset class="preferences">
          <legend class="sr-only">Preferences of {extension().name}</legend>
          <For each={extension().preferences}>
            {(preference) => (
              <PreferenceField
                extension={extension().name}
                preference={preference}
                onSet={props.onSet}
                onClear={props.onClear}
              />
            )}
          </For>
        </fieldset>
      </Show>
    </li>
  );
}

function PreferenceField(props: {
  extension: string;
  preference: Preference;
  onSet: (key: string, value: string) => Promise<void>;
  onClear: (key: string) => Promise<void>;
}) {
  const preference = () => props.preference;
  const name = () => `${preference().title} (${props.extension})`;
  const label = () => (
    <>
      {preference().title}
      <Show when={preference().required}>
        <span class="required"> (required)</span>
      </Show>
    </>
  );
  // A password is typed here and sent; it is never read back, so the field starts empty again.
  const [secret, setSecret] = createSignal("");
  return (
    <div class="preference" data-kind={preference().kind}>
      <Show
        when={preference().kind === "password"}
        fallback={
          <Show
            when={preference().kind === "checkbox"}
            fallback={
              <label>
                {label()}
                <input
                  type={preference().kind === "number" ? "number" : "text"}
                  aria-label={name()}
                  value={preference().value ?? ""}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    void (value === ""
                      ? props.onClear(preference().name)
                      : props.onSet(preference().name, value));
                  }}
                />
              </label>
            }
          >
            <label>
              <input
                type="checkbox"
                aria-label={name()}
                checked={preference().value === "true"}
                onChange={(event) =>
                  void props.onSet(preference().name, String(event.currentTarget.checked))
                }
              />
              {label()}
            </label>
          </Show>
        }
      >
        <label>
          {label()}
          <input
            type="password"
            autocomplete="off"
            aria-label={name()}
            placeholder={preference().is_set ? "Set" : "Not set"}
            value={secret()}
            onInput={(event) => setSecret(event.currentTarget.value)}
          />
        </label>
        <span class="secret-state">{preference().is_set ? "Set" : "Not set"}</span>
        <button
          type="button"
          aria-label={`Save ${name()}`}
          disabled={secret() === ""}
          onClick={() => {
            const value = secret();
            setSecret("");
            void props.onSet(preference().name, value);
          }}
        >
          Save
        </button>
        <Show when={preference().is_set}>
          <button
            type="button"
            aria-label={`Clear ${name()}`}
            onClick={() => void props.onClear(preference().name)}
          >
            Clear
          </button>
        </Show>
      </Show>
    </div>
  );
}
