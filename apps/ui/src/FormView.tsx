import { createSignal, For, Show } from "solid-js";
import type { FormField } from "./bindings";

/** The fields that are required but empty (after trimming). */
export function validate(
  fields: FormField[],
  values: Record<string, string>,
): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const field of fields) {
    if (field.required && (values[field.id] ?? "").trim() === "") errors[field.id] = "Required";
  }
  return errors;
}

/** A form view: validates on submit and shows the problem under the field. */
export function FormView(props: {
  title: string;
  fields: FormField[];
  onSubmit: (values: Record<string, string>) => void;
}) {
  const [values, setValues] = createSignal<Record<string, string>>({});
  const [errors, setErrors] = createSignal<Record<string, string>>({});

  const submit = (event: Event) => {
    event.preventDefault();
    const found = validate(props.fields, values());
    setErrors(found);
    if (Object.keys(found).length === 0) props.onSubmit(values());
  };

  return (
    <form class="form" aria-label={props.title} onSubmit={submit} novalidate>
      <For each={props.fields}>
        {(field) => (
          <div class="field">
            <label for={`field-${field.id}`}>{field.label}</label>
            <input
              id={`field-${field.id}`}
              name={field.id}
              aria-required={field.required}
              aria-invalid={errors()[field.id] !== undefined}
              value={values()[field.id] ?? ""}
              onInput={(event) => setValues({ ...values(), [field.id]: event.currentTarget.value })}
            />
            <Show when={errors()[field.id]}>
              <span class="error" role="alert">
                {errors()[field.id]}
              </span>
            </Show>
          </div>
        )}
      </For>
      <button type="submit">Submit</button>
    </form>
  );
}
