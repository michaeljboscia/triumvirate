<script lang="ts">
  // T-015 / T-020 — Root layout.
  //
  // Mounts app.css (Tailwind v4 + dark tokens) and boots the daemon
  // store. That's it.
  //
  // T-020 flicker fix (final): Pantheon is dark-only per REQ-029. The
  // previous versions of this file toggled a `.dark` class on <html>
  // via prefers-color-scheme matchMedia. That combined with Tauri v2's
  // default-white WKWebView paint gap to produce a rapid black/white
  // flicker on launch. Fix is three-part:
  //   1. tauri.conf.json sets backgroundColor=#1a1a1a + theme=Dark so
  //      the NATIVE window paints dark before the webview exists
  //   2. app.html has an inline <style> hard-pinning html/body dark,
  //      applied before any JS/CSS bundle loads
  //   3. app.css has `color-scheme: dark` on :root so browser-native
  //      controls (scrollbars, form elements) use dark from first paint
  // There is no runtime class toggling. There is no matchMedia. If the
  // user wants a light-mode Pantheon, that's a feature request for a
  // future sprint — the spec is dark-only for v4.0.

  import "../app.css";
  import { onMount } from "svelte";
  import { daemon } from "$lib/stores/daemon";

  let { children } = $props();

  onMount(() => {
    daemon.init();
    return () => daemon.destroy();
  });
</script>

{@render children()}
