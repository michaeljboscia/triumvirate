<script lang="ts">
  // T-015 (REQ-029) — Root layout. Mounts the app.css (Tailwind v4 + theme
  // tokens) and wires dark-mode auto-detection via prefers-color-scheme.
  //
  // Strategy: default to dark because Pantheon is a dev tool and that's
  // the expected resting state, but respect the OS preference on mount +
  // react live to appearance changes. Toggles the `.dark` class on
  // <html>, which app.css wires to the Tailwind `dark:` variant.
  //
  // Using $effect (not onMount) so the subscription cleanup runs
  // automatically when the layout unmounts — Svelte 5 pattern.

  import "../app.css";

  let { children } = $props();

  $effect(() => {
    if (typeof window === "undefined") return;

    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = (isDark: boolean) => {
      document.documentElement.classList.toggle("dark", isDark);
    };

    apply(mql.matches);
    const onChange = (e: MediaQueryListEvent) => apply(e.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  });
</script>

{@render children()}
