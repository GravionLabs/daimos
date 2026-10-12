import { fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { App } from "./App";
import { batch, extensionInfo, fakeHost, item, settingsView } from "./test-host";

async function openSettings(host: ReturnType<typeof fakeHost>) {
  render(() => <App host={host.host} />);
  host.openSettings();
  return screen.findByRole("region", { name: "Settings" });
}

describe("opening the settings", () => {
  it("opens from the button in the search row", async () => {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("region", { name: "Settings" })).toBeInTheDocument();
  });

  it("opens from the tray event and replaces the palette", async () => {
    const fake = fakeHost();
    const page = await openSettings(fake);
    expect(page).toBeInTheDocument();
    expect(screen.queryByRole("searchbox")).toBeNull();
    await waitFor(() => expect(fake.host.settings).toHaveBeenCalled());
  });

  it("toggles with Ctrl+, and Cmd+,", async () => {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    const box = screen.getByRole("searchbox");
    fireEvent.keyDown(box, { key: ",", ctrlKey: true });
    expect(await screen.findByRole("region", { name: "Settings" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("region", { name: "Settings" }), {
      key: ",",
      metaKey: true,
    });
    await waitFor(() => expect(screen.queryByRole("region", { name: "Settings" })).toBeNull());
    expect(screen.getByRole("searchbox")).toBeInTheDocument();
  });

  it("opens from the Settings command of the palette", async () => {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    fake.send(
      batch(0, [
        item("cmd:settings", {
          title: "Settings",
          actions: [
            {
              id: "open",
              title: "Open Settings",
              shortcut: "Enter",
              kind: "push",
              payload: "daimos:settings",
            },
          ],
        }),
      ]),
    );
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
    expect(await screen.findByRole("region", { name: "Settings" })).toBeInTheDocument();
    expect(fake.host.loadView).not.toHaveBeenCalled();
  });

  it("closes with Esc and with the back button, and on the next show", async () => {
    const fake = fakeHost();
    const page = await openSettings(fake);
    fireEvent.keyDown(page, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("region", { name: "Settings" })).toBeNull());
    expect(fake.host.hide).not.toHaveBeenCalled();

    fake.openSettings();
    fireEvent.click(await screen.findByRole("button", { name: "Back" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Settings" })).toBeNull());

    fake.openSettings();
    await screen.findByRole("region", { name: "Settings" });
    fake.show();
    await waitFor(() => expect(screen.queryByRole("region", { name: "Settings" })).toBeNull());
  });
});

describe("the settings page", () => {
  it("records a new hotkey and shows the conflict the system reports", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    const button = await screen.findByRole("button", { name: "Hotkey" });
    expect(button).toHaveTextContent("Alt+Space");

    vi.mocked(fake.host.setHotkey).mockResolvedValueOnce("Ctrl+Alt+K is taken by KWin");
    fireEvent.click(button);
    fireEvent.keyDown(button, { code: "KeyK", key: "k", ctrlKey: true, altKey: true });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("taken by KWin"));
    expect(fake.host.setHotkey).toHaveBeenCalledWith("Ctrl+Alt+K");

    vi.mocked(fake.host.settings).mockResolvedValueOnce(settingsView({ hotkey: "Ctrl+Alt+J" }));
    fireEvent.click(button);
    fireEvent.keyDown(button, { code: "KeyJ", key: "j", ctrlKey: true, altKey: true });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Hotkey" })).toHaveTextContent("Ctrl+Alt+J"),
    );
  });

  it("forces a theme and shows the stored one", async () => {
    const fake = fakeHost();
    vi.mocked(fake.host.settings).mockResolvedValue(settingsView({ theme: "dark" }));
    await openSettings(fake);
    expect(await screen.findByLabelText("Dark")).toBeChecked();
    expect(screen.getByLabelText("Automatic")).not.toBeChecked();
    fireEvent.click(screen.getByLabelText("Light"));
    expect(fake.host.setTheme).toHaveBeenCalledExactlyOnceWith("light");
  });

  it("switches providers off and on and sets their weight", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    const apps = await screen.findByLabelText("apps");
    expect(apps).toBeChecked();
    fireEvent.click(apps);
    expect(fake.host.setProvider).toHaveBeenLastCalledWith("apps", false, null);

    const weight = screen.getByLabelText("Weight of calc");
    expect(weight).toHaveValue(10);
    fireEvent.change(weight, { target: { value: "2.5" } });
    expect(fake.host.setProvider).toHaveBeenLastCalledWith("calc", true, 2.5);
    // Back to the provider's own weight clears the override.
    fireEvent.change(weight, { target: { value: "10" } });
    expect(fake.host.setProvider).toHaveBeenLastCalledWith("calc", true, null);
  });

  it("adds and removes aliases", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    await screen.findByText("Firefox");

    fireEvent.input(screen.getByLabelText("Alias"), { target: { value: "term" } });
    fireEvent.input(screen.getByLabelText("Search for"), { target: { value: "Terminal" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    expect(fake.host.setAliases).toHaveBeenLastCalledWith([
      { alias: "ff", target: "Firefox" },
      { alias: "term", target: "Terminal" },
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Remove ff" }));
    expect(fake.host.setAliases).toHaveBeenLastCalledWith([]);
  });

  it("does not add an incomplete alias", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    const add = await screen.findByRole("button", { name: "Add" });
    expect(add).toBeDisabled();
    fireEvent.input(screen.getByLabelText("Alias"), { target: { value: "x" } });
    expect(add).toBeDisabled();
    expect(fake.host.setAliases).not.toHaveBeenCalled();
  });

  it("lists the capabilities with their level and reason", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    const hotkey = (await screen.findByText("Global hotkey")).closest("li") as HTMLElement;
    expect(hotkey).toHaveAttribute("data-level", "available");
    expect(within(hotkey).getByText("Available")).toBeInTheDocument();
    expect(hotkey).toHaveTextContent("Registered through XGrabKey.");
    const paste = screen.getByText("Paste into the previous window").closest("li") as HTMLElement;
    expect(within(paste).getByText("Unavailable")).toBeInTheDocument();
  });

  it("shows the compositor instructions when there are some", async () => {
    const fake = fakeHost();
    vi.mocked(fake.host.capabilities).mockResolvedValue({
      session: "wayland",
      window_mode: "toplevel",
      hotkey: { state: "unsupported", session: "wayland", binding: null, message: "no portal" },
      items: [
        {
          id: "hotkey",
          label: "Global hotkey",
          level: "unavailable",
          reason: "no portal",
          instructions:
            "Bind the command `daimos toggle`\n  sway       bindsym Mod1+space exec daimos toggle",
        },
      ],
    });
    await openSettings(fake);
    expect(await screen.findByText(/bindsym Mod1\+space exec daimos toggle/)).toBeInTheDocument();
    expect(screen.getByText("Unavailable")).toBeInTheDocument();
  });

  it("shows a failure instead of an empty page", async () => {
    const fake = fakeHost();
    vi.mocked(fake.host.settings).mockRejectedValue(new Error("settings database is locked"));
    await openSettings(fake);
    expect(await screen.findByRole("alert")).toHaveTextContent("settings database is locked");
  });
});

describe("the clipboard history setting", () => {
  it("is off by default and switches on with the checkbox", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    const box = await screen.findByRole("checkbox", { name: "Remember what I copy" });
    expect(box).not.toBeChecked();
    fireEvent.click(box);
    await waitFor(() => expect(fake.host.setClipboardHistory).toHaveBeenCalledWith(true));
    await waitFor(() => expect(box).toBeChecked());
  });

  it("says why it is limited, and cannot be switched on when unavailable", async () => {
    const fake = fakeHost();
    fake.host.settings = vi.fn(() =>
      Promise.resolve(
        settingsView({
          clipboard: {
            enabled: false,
            max_entries: 500,
            max_image_mb: 10,
            level: "unavailable",
            reason: "no X server",
          },
        }),
      ),
    );
    await openSettings(fake);
    expect(await screen.findByText("no X server")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Remember what I copy" })).toBeDisabled();
  });
});

describe("the extensions section", () => {
  const withExtensions = (
    fake: ReturnType<typeof fakeHost>,
    ...list: ReturnType<typeof extensionInfo>[]
  ) => vi.mocked(fake.host.extensions).mockResolvedValue({ extensions: list });

  it("says when no extension is installed", async () => {
    const fake = fakeHost();
    await openSettings(fake);
    expect(await screen.findByText("No extensions installed.")).toBeInTheDocument();
  });

  it("labels scripts as running unsandboxed and wasm as sandboxed", async () => {
    const fake = fakeHost();
    withExtensions(
      fake,
      extensionInfo({ name: "notes", title: "Notes" }),
      extensionInfo({
        name: "repos",
        title: "Repos",
        runtime: "wasm",
        sandboxed: true,
        version: "0.2.0",
      }),
    );
    await openSettings(fake);
    const notes = (await screen.findByText("Notes")).closest("li") as HTMLElement;
    expect(within(notes).getByText("Runs unsandboxed")).toBeInTheDocument();
    expect(within(notes).queryByText("Sandboxed")).toBeNull();
    expect(within(notes).getByText("1.0.0")).toBeInTheDocument();
    expect(within(notes).getByText("Enabled")).toBeInTheDocument();
    const repos = screen.getByText("Repos").closest("li") as HTMLElement;
    expect(within(repos).getByText("Sandboxed")).toBeInTheDocument();
    expect(within(repos).queryByText("Runs unsandboxed")).toBeNull();
  });

  it("lists the capabilities of an extension that waits for a confirmation and confirms it", async () => {
    const fake = fakeHost();
    withExtensions(
      fake,
      extensionInfo({
        state: "needs-confirmation",
        capabilities: {
          network: ["api.example.com"],
          filesystem: [
            { path: "$HOME/.config/gh", write: false },
            { path: "/tmp/x", write: true },
          ],
          clipboard: "read-write",
          keyring: ["token"],
        },
        added: {
          network: ["api.example.com"],
          filesystem: [
            { path: "$HOME/.config/gh", write: false },
            { path: "/tmp/x", write: true },
          ],
          clipboard: "read-write",
          keyring: ["token"],
        },
      }),
    );
    await openSettings(fake);
    const list = await screen.findByRole("list", { name: "Capabilities of notes" });
    const lines = within(list)
      .getAllByRole("listitem")
      .map((li) => li.textContent);
    expect(lines).toEqual([
      "Files: $HOME/.config/gh (read only)",
      "Files: /tmp/x (read and write)",
      "Network: api.example.com",
      "Clipboard: read and write",
      "Keyring: token",
    ]);
    // The first confirmation compares with nothing: nothing is marked as new.
    expect(within(list).queryByText("(new)")).toBeNull();
    expect(screen.getByText("Needs confirmation")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Confirm notes" }));
    expect(fake.host.confirmExtension).toHaveBeenCalledExactlyOnceWith("notes");
    // The list is read again after the change.
    await waitFor(() => expect(fake.host.extensions).toHaveBeenCalledTimes(2));
  });

  it("marks what an update added to the confirmed capabilities", async () => {
    const fake = fakeHost();
    withExtensions(
      fake,
      extensionInfo({
        state: "needs-confirmation",
        capabilities: { network: ["a.com", "b.com"], filesystem: [], keyring: ["k"] },
        confirmed: { network: ["a.com"], filesystem: [], keyring: ["k"] },
        added: { network: ["b.com"], filesystem: [], keyring: [] },
      }),
    );
    await openSettings(fake);
    const list = await screen.findByRole("list", { name: "Capabilities of notes" });
    const [a, b, k] = within(list).getAllByRole("listitem");
    expect(a).toHaveTextContent("Network: a.com");
    expect(a).not.toHaveTextContent("(new)");
    expect(b).toHaveTextContent("Network: b.com (new)");
    expect(k).not.toHaveTextContent("(new)");
    expect(screen.getByText(/A new version asks for more/)).toBeInTheDocument();
  });

  it("shows no confirm button for an enabled extension and says when nothing is declared", async () => {
    const fake = fakeHost();
    withExtensions(fake, extensionInfo());
    await openSettings(fake);
    expect(await screen.findByText("It declares no capabilities.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Confirm/ })).toBeNull();
  });

  it("shows the message of a broken extension", async () => {
    const fake = fakeHost();
    withExtensions(
      fake,
      extensionInfo({
        title: null,
        runtime: null,
        state: "error",
        message: "extension.toml:2: bad",
      }),
    );
    await openSettings(fake);
    expect(await screen.findByText("extension.toml:2: bad")).toBeInTheDocument();
    expect(screen.queryByText("Runs unsandboxed")).toBeNull();
  });

  it("shows a failed confirmation", async () => {
    const fake = fakeHost();
    withExtensions(fake, extensionInfo({ state: "needs-confirmation" }));
    vi.mocked(fake.host.confirmExtension).mockRejectedValue(new Error("cannot record the consent"));
    await openSettings(fake);
    fireEvent.click(await screen.findByRole("button", { name: "Confirm notes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot record the consent");
  });

  describe("preferences", () => {
    const preferences = [
      { name: "token", title: "Token", kind: "password", required: true, is_set: false },
      { name: "user", title: "User", kind: "text", required: false, is_set: true, value: "ada" },
      { name: "limit", title: "Limit", kind: "number", required: false, is_set: false },
      {
        name: "drafts",
        title: "Drafts",
        kind: "checkbox",
        required: false,
        is_set: true,
        value: "true",
      },
    ];

    it("renders one field per type and marks the required ones", async () => {
      const fake = fakeHost();
      withExtensions(fake, extensionInfo({ preferences }));
      await openSettings(fake);
      const group = await screen.findByRole("group", { name: "Preferences of notes" });
      const token = within(group).getByLabelText("Token (notes)");
      expect(token).toHaveAttribute("type", "password");
      expect(within(group).getByText("(required)")).toBeInTheDocument();
      expect(within(group).getAllByText("(required)")).toHaveLength(1);
      expect(within(group).getByLabelText("User (notes)")).toHaveValue("ada");
      expect(within(group).getByLabelText("Limit (notes)")).toHaveAttribute("type", "number");
      expect(within(group).getByLabelText("Drafts (notes)")).toBeChecked();
    });

    it("stores text, number and checkbox values and clears an emptied field", async () => {
      const fake = fakeHost();
      withExtensions(fake, extensionInfo({ preferences }));
      await openSettings(fake);
      fireEvent.change(await screen.findByLabelText("User (notes)"), {
        target: { value: "grace" },
      });
      expect(fake.host.setPreference).toHaveBeenLastCalledWith("notes", "user", "grace");
      fireEvent.change(screen.getByLabelText("Limit (notes)"), { target: { value: "12" } });
      expect(fake.host.setPreference).toHaveBeenLastCalledWith("notes", "limit", "12");
      fireEvent.click(screen.getByLabelText("Drafts (notes)"));
      expect(fake.host.setPreference).toHaveBeenLastCalledWith("notes", "drafts", "false");
      fireEvent.change(screen.getByLabelText("User (notes)"), { target: { value: "" } });
      expect(fake.host.clearPreference).toHaveBeenLastCalledWith("notes", "user");
    });

    it("never echoes a password: the field is empty and only says whether one is set", async () => {
      const fake = fakeHost();
      withExtensions(
        fake,
        extensionInfo({
          preferences: [
            { name: "token", title: "Token", kind: "password", required: true, is_set: true },
          ],
        }),
      );
      await openSettings(fake);
      const field = await screen.findByLabelText("Token (notes)");
      expect(field).toHaveValue("");
      expect(field).toHaveAttribute("placeholder", "Set");
      expect(screen.getByText("Set", { selector: ".secret-state" })).toBeInTheDocument();

      expect(screen.getByRole("button", { name: "Save Token (notes)" })).toBeDisabled();
      fireEvent.input(field, { target: { value: "hunter2" } });
      fireEvent.click(screen.getByRole("button", { name: "Save Token (notes)" }));
      expect(fake.host.setPreference).toHaveBeenCalledExactlyOnceWith("notes", "token", "hunter2");
      await waitFor(() => expect(screen.getByLabelText("Token (notes)")).toHaveValue(""));
      expect(document.body.innerHTML).not.toContain("hunter2");

      fireEvent.click(screen.getByRole("button", { name: "Clear Token (notes)" }));
      expect(fake.host.clearPreference).toHaveBeenCalledExactlyOnceWith("notes", "token");
    });

    it("offers no Clear for a password that is not set", async () => {
      const fake = fakeHost();
      withExtensions(fake, extensionInfo({ preferences: [preferences[0] as never] }));
      await openSettings(fake);
      await screen.findByLabelText("Token (notes)");
      expect(screen.getByText("Not set", { selector: ".secret-state" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /Clear/ })).toBeNull();
    });
  });
});
