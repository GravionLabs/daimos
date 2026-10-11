import { createSignal, For, onMount, Show } from "solid-js";
import type { Capabilities, Level, SettingsView as Settings, ThemeSetting } from "./bindings";
import { HotkeyRecorder } from "./HotkeyRecorder";
import type { Host } from "./host";

/** Weights are floats: a missing one (null) is the provider's own. */
const weightOf = (provider: { weight: number | null; default_weight: number | null }) =>
  provider.weight ?? provider.default_weight ?? 1;

const LEVEL_LABEL: Record<Level, string> = {
  available: "Available",
  limited: "Limited",
  unavailable: "Unavailable",
};

const THEMES: { value: ThemeSetting; label: string }[] = [
  { value: "system", label: "Automatic" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** The settings page: hotkey, appearance, providers, aliases and what this system can do. */
export function SettingsView(props: { host: Host; onClose: () => void }) {
  const [settings, setSettings] = createSignal<Settings | null>(null);
  const [capabilities, setCapabilities] = createSignal<Capabilities | null>(null);
  const [problem, setProblem] = createSignal<string | null>(null);

  const load = async () => {
    try {
      setSettings(await props.host.settings());
      setCapabilities(await props.host.capabilities());
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    }
  };
  onMount(() => void load());

  /** Runs a change and shows what the core stored; a failure stays visible. */
  const change = async (action: () => Promise<Settings>) => {
    try {
      setSettings(await action());
      setProblem(null);
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    }
  };

  const setAliases = (aliases: { alias: string; target: string }[]) =>
    change(() => props.host.setAliases(aliases));

  return (
    <section class="settings" aria-label="Settings">
      <header class="crumb">
        <button type="button" class="back" aria-label="Back" onClick={props.onClose}>
          ←
        </button>
        <span>Settings</span>
      </header>
      <Show when={problem()}>
        <p class="error" role="alert">
          {problem()}
        </p>
      </Show>
      <Show when={settings()}>
        {(current) => (
          <div class="settings-body">
            <h3>Hotkey</h3>
            <HotkeyRecorder
              binding={current().hotkey}
              onRecord={async (accelerator) => {
                const error = await props.host.setHotkey(accelerator);
                if (!error) await load();
                return error;
              }}
            />

            <h3>Appearance</h3>
            <fieldset class="themes">
              <legend class="sr-only">Theme</legend>
              <For each={THEMES}>
                {(theme) => (
                  <label>
                    <input
                      type="radio"
                      name="theme"
                      checked={current().theme === theme.value}
                      onChange={() => void change(() => props.host.setTheme(theme.value))}
                    />
                    {theme.label}
                  </label>
                )}
              </For>
            </fieldset>

            <h3>Providers</h3>
            <ul class="providers">
              <For each={current().providers}>
                {(provider) => (
                  <li>
                    <label>
                      <input
                        type="checkbox"
                        checked={provider.enabled}
                        onChange={(event) =>
                          void change(() =>
                            props.host.setProvider(
                              provider.id,
                              event.currentTarget.checked,
                              weightOf(provider) === (provider.default_weight ?? 1)
                                ? null
                                : weightOf(provider),
                            ),
                          )
                        }
                      />
                      {provider.id}
                    </label>
                    <label class="weight">
                      Weight
                      <input
                        type="number"
                        step="0.5"
                        min="0"
                        aria-label={`Weight of ${provider.id}`}
                        value={weightOf(provider)}
                        onChange={(event) => {
                          const weight = Number.parseFloat(event.currentTarget.value);
                          void change(() =>
                            props.host.setProvider(
                              provider.id,
                              provider.enabled,
                              Number.isFinite(weight) && weight !== (provider.default_weight ?? 1)
                                ? weight
                                : null,
                            ),
                          );
                        }}
                      />
                    </label>
                  </li>
                )}
              </For>
            </ul>

            <h3>Aliases</h3>
            <p class="hint">
              Typing the alias searches for what it stands for, for example ff for Firefox.
            </p>
            <ul class="aliases">
              <For each={current().aliases}>
                {(entry) => (
                  <li>
                    <span class="alias">{entry.alias}</span>
                    <span aria-hidden="true">→</span>
                    <span class="target">{entry.target}</span>
                    <button
                      type="button"
                      aria-label={`Remove ${entry.alias}`}
                      onClick={() =>
                        void setAliases(
                          current().aliases.filter((other) => other.alias !== entry.alias),
                        )
                      }
                    >
                      ×
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <AliasForm
              onAdd={(alias, target) =>
                setAliases([
                  ...current().aliases.filter((other) => other.alias !== alias.toLowerCase()),
                  { alias, target },
                ])
              }
            />

            <h3>Platform</h3>
            <ul class="capabilities">
              <For each={capabilities()?.items ?? []}>
                {(item) => (
                  <li data-level={item.level}>
                    <strong>{item.label}</strong>
                    <span class="level">{LEVEL_LABEL[item.level]}</span>
                    <p>{item.reason}</p>
                    <Show when={item.instructions}>
                      <pre>{item.instructions}</pre>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </div>
        )}
      </Show>
    </section>
  );
}

function AliasForm(props: { onAdd: (alias: string, target: string) => void }) {
  const [alias, setAlias] = createSignal("");
  const [target, setTarget] = createSignal("");
  const valid = () => alias().trim() !== "" && target().trim() !== "";
  return (
    <form
      class="alias-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid()) return;
        props.onAdd(alias().trim(), target().trim());
        setAlias("");
        setTarget("");
      }}
    >
      <input
        aria-label="Alias"
        placeholder="ff"
        value={alias()}
        onInput={(event) => setAlias(event.currentTarget.value)}
      />
      <input
        aria-label="Search for"
        placeholder="Firefox"
        value={target()}
        onInput={(event) => setTarget(event.currentTarget.value)}
      />
      <button type="submit" disabled={!valid()}>
        Add
      </button>
    </form>
  );
}
