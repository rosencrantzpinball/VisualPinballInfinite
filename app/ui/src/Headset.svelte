<script>
  // The app's screens in the headset (`#headset`): shown by the player's panel (vpi-panel, PLAN
  // decision 20) on a panel in the room. The library and a table's page as in the app, under a bar of
  // their own: the library, back to the table played last (when there is one), and quit. PLAY starts
  // the table in the headset (the player starts again with it).
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { app, start, notice, dismiss, goLibrary } from './lib/store.svelte.js';
  import Library from './lib/Library.svelte';
  import TablePage from './lib/TablePage.svelte';
  import Icon from './lib/Icon.svelte';

  // the table the player goes back to (its path), if any
  let back = $state(null);
  // named as the library names it once read, else by its file
  const backName = $derived(
    back && (app.tables.find((t) => t.path === back)?.short ?? back.split(/[\\/]/).pop().replace(/\.vpx$/i, ''))
  );

  onMount(() => {
    start().catch((e) => notice(`The library couldn’t be read: ${e}`, { kind: 'problem' }));
    invoke('headset').then((h) => (back = h?.back ?? null)).catch(() => {});
  });

  const ask = (command) => invoke(command).catch((e) => notice(String(e), { kind: 'problem' }));
</script>

<div class="window">
  <header class="bar">
    <button type="button" class="btn ghost title" onclick={goLibrary}>
      <Icon name="grid" size={22} />
      <span>Visual Pinball Infinite</span>
    </button>
    {#if app.status.phase === 'reading'}
      <span class="reading">Reading the tables… {app.status.done} of {app.status.of}</span>
    {/if}
    <span class="space"></span>
    {#if back}
      <button type="button" class="btn" onclick={() => ask('panel_close')}>Back to {backName}</button>
    {/if}
    <button type="button" class="btn ghost" onclick={() => ask('quit')}>Quit</button>
  </header>
  <div class="screen">
    {#if app.page === 'table'}
      <TablePage />
    {:else}
      <Library />
    {/if}
  </div>
  {#if app.notices.length}
    <div class="notices" role="status" aria-live="polite">
      {#each app.notices as n (n.id)}
        <div class="notice {n.kind}">
          <span class="text">{n.text}</span>
          <button type="button" class="btn small ghost close" aria-label="Close" onclick={() => dismiss(n.id)}><Icon name="x" size={16} /></button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .window { min-height: 100%; display: flex; flex-direction: column; }
  .bar {
    position: sticky; top: 0; z-index: 40; display: flex; align-items: center; gap: 12px;
    padding: 10px 16px; background: rgba(12, 14, 20, 0.92); border-bottom: 1px solid #222a3a;
  }
  .title { display: inline-flex; align-items: center; gap: 10px; font-size: 18px; font-weight: 700; }
  .reading { color: var(--fg-soft); font-size: 14px; }
  .space { flex: 1; }
  .screen { flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .notices {
    position: fixed; right: 20px; bottom: 20px; z-index: 50;
    display: flex; flex-direction: column; gap: 10px; max-width: min(560px, calc(100vw - 40px));
  }
  .notice {
    display: flex; align-items: center; gap: 12px; padding: 14px 14px 14px 18px; font-size: 16px;
    border-radius: 12px; background: #182031; border: 1px solid #2a3550; box-shadow: 0 12px 30px rgba(0, 0, 0, 0.5);
  }
  .notice.problem { background: #2a1820; border-color: #5a2a3a; }
  .notice .text { flex: 1; }
  .close { width: 36px; padding: 0; }
  /* pointed at with a controller: bigger targets than a mouse needs */
  :global(html.headset .btn) { min-height: 44px; }
</style>
