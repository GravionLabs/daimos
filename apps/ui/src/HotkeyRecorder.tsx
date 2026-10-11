import { createSignal, Show } from "solid-js";
import { acceleratorFromEvent, isMac } from "./hotkey";

/**
 * Shows the current shortcut and records a new one: press the button, then the keys. The system
 * decides whether the shortcut is free; its answer is shown under the button.
 */
export function HotkeyRecorder(props: {
  binding: string;
  /** Tries the shortcut; resolves to an error message, or null when it was set. */
  onRecord: (accelerator: string) => Promise<string | null>;
}) {
  const [recording, setRecording] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const finish = async (accelerator: string) => {
    setRecording(false);
    setError(await props.onRecord(accelerator));
  };

  return (
    <div class="recorder">
      <button
        type="button"
        class="recorder-button"
        aria-label="Hotkey"
        data-recording={recording()}
        onClick={() => {
          setError(null);
          setRecording(true);
        }}
        onBlur={() => setRecording(false)}
        onKeyDown={(event) => {
          if (!recording()) return;
          // The palette's own keys must not run while a shortcut is recorded.
          event.preventDefault();
          event.stopPropagation();
          if (event.key === "Escape") return setRecording(false);
          const accelerator = acceleratorFromEvent(event, isMac());
          if (accelerator) void finish(accelerator);
        }}
      >
        {recording() ? "Press the new shortcut…" : props.binding}
      </button>
      <Show when={error()}>
        <span class="error" role="alert">
          {error()}
        </span>
      </Show>
    </div>
  );
}
