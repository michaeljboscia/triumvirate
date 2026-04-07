import "./app.css";
import { mount } from "svelte";
import App from "./routes/App.svelte";

const app = mount(App, {
	target: document.getElementById("app")!,
});

export default app;
