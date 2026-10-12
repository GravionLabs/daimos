import { For, Match, Show, Switch } from "solid-js";
import type { Detail } from "./bindings";
import { type Block, type Inline, parseMarkdown } from "./markdown";
import { UNSANDBOXED_LABEL } from "./results";

function Inlines(props: { inlines: Inline[] }) {
  return (
    <For each={props.inlines}>
      {(inline) => {
        if (inline.code) return <code>{inline.text}</code>;
        if (inline.bold) return <strong>{inline.text}</strong>;
        if (inline.italic) return <em>{inline.text}</em>;
        return inline.text;
      }}
    </For>
  );
}

function BlockView(props: { block: Block }) {
  const block = () => props.block;
  return (
    <Switch>
      <Match when={block().type === "heading" && (block() as Extract<Block, { type: "heading" }>)}>
        {(heading) => (
          <p class="md-heading" data-level={heading().level}>
            <Inlines inlines={heading().inlines} />
          </p>
        )}
      </Match>
      <Match
        when={block().type === "paragraph" && (block() as Extract<Block, { type: "paragraph" }>)}
      >
        {(paragraph) => (
          <p>
            <Inlines inlines={paragraph().inlines} />
          </p>
        )}
      </Match>
      <Match when={block().type === "list" && (block() as Extract<Block, { type: "list" }>)}>
        {(list) => (
          <ul>
            <For each={list().items}>
              {(inlines) => (
                <li>
                  <Inlines inlines={inlines} />
                </li>
              )}
            </For>
          </ul>
        )}
      </Match>
      <Match when={block().type === "code" && (block() as Extract<Block, { type: "code" }>)}>
        {(code) => <pre>{code().text}</pre>}
      </Match>
    </Switch>
  );
}

/** The detail of an item: Markdown text, key/value metadata or a picture. */
export function DetailPane(props: {
  detail: Detail | null;
  /** The item comes from a script extension: the header says so. */
  unsandboxed?: boolean;
}) {
  return (
    <aside class="detail" aria-label="Detail">
      <Show when={props.unsandboxed}>
        <p class="detail-header">
          <span class="badge" data-sandboxed="false">
            {UNSANDBOXED_LABEL}
          </span>
        </p>
      </Show>
      <Switch fallback={<p class="empty">No detail</p>}>
        <Match
          when={
            props.detail?.type === "markdown" &&
            (props.detail as Extract<Detail, { type: "markdown" }>)
          }
        >
          {(markdown) => (
            <For each={parseMarkdown(markdown().text)}>
              {(block) => <BlockView block={block} />}
            </For>
          )}
        </Match>
        <Match
          when={
            props.detail?.type === "image" && (props.detail as Extract<Detail, { type: "image" }>)
          }
        >
          {(image) => (
            <figure class="detail-image">
              <img src={image().data_url} alt={image().caption} />
              <figcaption>{image().caption}</figcaption>
            </figure>
          )}
        </Match>
        <Match
          when={
            props.detail?.type === "metadata" &&
            (props.detail as Extract<Detail, { type: "metadata" }>)
          }
        >
          {(metadata) => (
            <dl>
              <For each={metadata().entries}>
                {([key, value]) => (
                  <>
                    <dt>{key}</dt>
                    <dd>{value}</dd>
                  </>
                )}
              </For>
            </dl>
          )}
        </Match>
      </Switch>
    </aside>
  );
}
