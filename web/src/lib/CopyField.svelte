<script>
  export let label = "";
  export let value = "";
  let copied = false;
  let timer;

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Fallback for non-secure contexts.
      const ta = document.createElement("textarea");
      ta.value = value;
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
      } catch {}
      document.body.removeChild(ta);
    }
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), 1400);
  }
</script>

<div class="copy-field">
  {#if label}<span class="lbl">{label}</span>{/if}
  <div class="row">
    <code class="mono val">{value}</code>
    <button class="btn-ghost btn-sm" on:click={copy} title="Copy">
      {copied ? "Copied ✓" : "Copy"}
    </button>
  </div>
</div>

<style>
  .copy-field {
    margin-bottom: 12px;
  }
  .lbl {
    display: block;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
    margin-bottom: 5px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .val {
    flex: 1;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 9px 12px;
    font-size: 14px;
    overflow-x: auto;
    white-space: nowrap;
  }
  .row button {
    flex-shrink: 0;
  }
</style>
