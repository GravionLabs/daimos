import { fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { App } from "./App";
import { batch, fakeHost, item, settingsView } from "./test-host";

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
