/* @refresh reload */
import { render } from "solid-js/web";
import { App } from "./App";
import "./styles.css";

// First paint happens here, before any data is requested from the Rust core.
const root = document.getElementById("root");
if (root) render(() => <App />, root);
