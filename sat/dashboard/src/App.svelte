<script>
  import { onMount } from 'svelte';
  import { slices, colors, duration, commandText } from './chart.js';

  let overview = $state(null);
  let selected = $state('');
  let status = $state('');
  let search = $state('');
  let page = $state(0);
  let refresh = $state(0);
  let records = $state({ total: 0, items: [] });
  let loading = $state(true);
  let error = $state('');
  let detail = $state(null);
  let help = $state(false);
  let copied = $state('');
  const pageSize = 25;
  const chart = $derived(slices(overview?.tools ?? []));
  const totals = $derived(
    overview?.totals ?? {
      executions: 0,
      successes: 0,
      failures: 0,
      running: 0,
      tools: 0,
      duration_ms: 0,
    },
  );
  const successRate = $derived(
    totals.successes + totals.failures
      ? (
          (100 * totals.successes) /
          (totals.successes + totals.failures)
        ).toFixed(1)
      : '—',
  );
  const activity = $derived.by(() => {
    const entries = overview?.daily ?? [];
    return Array.from({ length: 7 }, (_, i) => {
      const date = new Date();
      date.setUTCDate(date.getUTCDate() - 6 + i);
      const day = date.toISOString().slice(0, 10);
      return {
        day,
        label: date.toLocaleDateString(undefined, {
          weekday: 'short',
          timeZone: 'UTC',
        }),
        count: entries.find((d) => d.day === day)?.count ?? 0,
      };
    });
  });
  const maxActivity = $derived(Math.max(1, ...activity.map((d) => d.count)));

  async function get(url, signal) {
    const response = await fetch(url, { signal });
    if (!response.ok)
      throw new Error(
        `Dashboard request failed (${response.status}). Try refreshing.`,
      );
    return response.json();
  }
  $effect(() => {
    refresh;
    const controller = new AbortController();
    get('/api/overview', controller.signal)
      .then((data) => {
        overview = data;
        error = '';
      })
      .catch((e) => {
        if (e.name !== 'AbortError') error = e.message;
      });
    return () => controller.abort();
  });
  $effect(() => {
    refresh;
    const controller = new AbortController();
    const params = new URLSearchParams({
      tool: selected,
      status,
      q: search,
      offset: String(page * pageSize),
      limit: String(pageSize),
    });
    loading = true;
    get(`/api/executions?${params}`, controller.signal)
      .then((data) => {
        records = data;
        loading = false;
      })
      .catch((e) => {
        if (e.name !== 'AbortError') {
          error = e.message;
          loading = false;
        }
      });
    return () => controller.abort();
  });
  onMount(() => {
    const timer = setInterval(() => refresh++, 10000);
    return () => clearInterval(timer);
  });
  function choose(tool) {
    selected = selected === tool ? '' : tool;
    page = 0;
  }
  function date(value) {
    return new Date(value).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  }
  function toolColor(tool) {
    const index = overview?.tools.findIndex((t) => t.name === tool) ?? 0;
    return colors[Math.max(0, index) % colors.length];
  }
  async function copy(text, label) {
    try {
      await navigator.clipboard.writeText(text);
      copied = label;
      setTimeout(() => (copied = ''), 2000);
    } catch {
      copied = 'Copy unavailable';
    }
  }
  function focusModal(node) {
    const previous = document.activeElement;
    node.focus();
    function trap(event) {
      if (event.key !== 'Tab') return;
      const items = [
        ...node.querySelectorAll(
          'button, a[href], input, select, [tabindex="0"]',
        ),
      ].filter((el) => !el.disabled);
      const first = items[0],
        last = items.at(-1);
      if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === node)
      ) {
        event.preventDefault();
        last?.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === node)
      ) {
        event.preventDefault();
        first?.focus();
      }
    }
    node.addEventListener('keydown', trap);
    return {
      destroy() {
        node.removeEventListener('keydown', trap);
        previous?.focus();
      },
    };
  }
  function closeOnEscape(event) {
    if (event.key === 'Escape') {
      detail = null;
      help = false;
    }
  }
</script>

<svelte:window onkeydown={closeOnEscape} />
<svelte:head
  ><meta
    name="description"
    content="Your local SAT tool activity: executions, performance, and command history."
  /></svelte:head
>

<div class="shell">
  <aside class="sidebar">
    <a class="brand" href="#overview" aria-label="SAT dashboard home"
      ><span class="brand-icon">›_</span><span
        >SAT<span class="brand-sub">SANDBOXED AGENT TOOLS</span></span
      ></a
    >
    <div class="workspace">
      <span class="workspace-icon">⌘</span>
      <div>Local workspace<small>Execution dashboard</small></div>
      <span class="local-dot"></span>
    </div>
    <div class="nav-label">WORKSPACE</div>
    <nav aria-label="Main navigation">
      <a href="#overview" class="nav-item active"
        ><span>◫</span> Overview <span class="nav-dot"></span></a
      >
      <a href="#executions" class="nav-item"
        ><span>≡</span> Executions
        <span class="nav-count">{totals.executions.toLocaleString()}</span></a
      >
      <button class="nav-item" onclick={() => (help = true)}
        ><span>⚙</span> History settings</button
      >
    </nav>
    <div class="sidebar-bottom">
      <div class="privacy-card">
        <div class="privacy-symbol">◇</div>
        <strong>Your tools. Your data.</strong>
        <p>Execution history stays on this machine, in your SAT home.</p>
      </div>
      <div class="sidebar-footer">
        <span class="avatar">S</span>
        <div>SAT runtime<small>Local · private by default</small></div>
        <span class="local-dot"></span>
      </div>
    </div>
  </aside>

  <div class="main-shell">
    <header class="topbar">
      <div>
        <span class="breadcrumb">Workspace</span><span class="slash">/</span
        >Overview
      </div>
      <div class="topbar-right">
        <span class="localhost"><span class="local-dot"></span> localhost</span
        ><span class="topbar-divider"></span><button
          class="icon-button"
          aria-label="History settings"
          onclick={() => (help = true)}>⚙</button
        >
      </div>
    </header>
    <main id="overview">
      <div class="page-heading">
        <div>
          <div class="eyebrow">A LITTLE VISIBILITY GOES A LONG WAY</div>
          <h1>Tool activity<span>.</span></h1>
          <p>A clear view of what your agents are running.</p>
        </div>
        <div class="heading-actions">
          <span class:disabled={!overview?.enabled} class="recording-pill"
            ><span></span>{overview?.enabled
              ? 'Recording enabled'
              : 'Recording disabled'}</span
          ><button
            class="button"
            onclick={() => refresh++}
            aria-label="Refresh dashboard"
            ><span class:spinning={loading}>↻</span> Refresh</button
          >
        </div>
      </div>

      {#if error}<div class="error-banner" role="alert">
          {error}<button onclick={() => refresh++}>Retry ↗</button>
        </div>{/if}
      {#if overview && !overview.enabled}<div class="notice">
          <span>◉</span>
          <div>
            <strong>History recording is off.</strong> Enable it to see future tool
            executions here. Existing history is still available.
          </div>
          <button
            class="code-button"
            onclick={() => copy('sat dashboard enable', 'enable')}
            >{copied === 'enable' ? 'Copied ✓' : 'sat dashboard enable'}
            <span>⧉</span></button
          >
        </div>{/if}

      <section class="stats" aria-label="Execution statistics">
        <article class="stat">
          <div class="stat-label">
            Total executions <span class="stat-icon lavender">↗</span>
          </div>
          <div class="stat-value">{totals.executions.toLocaleString()}</div>
          <div class="stat-note">
            <span class="subtle-dot purple"></span> Across your recorded history
          </div>
        </article>
        <article class="stat">
          <div class="stat-label">
            Active tools <span class="stat-icon blue">⊞</span>
          </div>
          <div class="stat-value">{totals.tools.toLocaleString()}</div>
          <div class="stat-note">Unique tools in your workspace</div>
        </article>
        <article class="stat">
          <div class="stat-label">
            Success rate <span class="stat-icon mint">✓</span>
          </div>
          <div class="stat-value">
            {successRate}<span class="value-unit"
              >{successRate === '—' ? '' : '%'}</span
            >
          </div>
          <div class="stat-note">
            <span class="subtle-dot green"></span>
            {totals.failures} failed · {totals.running} unfinished
          </div>
        </article>
        <article class="stat">
          <div class="stat-label">
            Total runtime <span class="stat-icon peach">◷</span>
          </div>
          <div class="stat-value duration-value">
            {duration(totals.duration_ms)}
          </div>
          <div class="stat-note">Wall time of completed invocations</div>
        </article>
      </section>

      <div class="charts-grid">
        <section class="panel usage-panel" aria-labelledby="usage-title">
          <div class="panel-heading">
            <div>
              <h2 id="usage-title">Tool distribution</h2>
              <p>Every execution, a piece of the picture.</p>
            </div>
            <span class="small-badge">All time</span>
          </div>
          {#if chart.length}
            <div class="distribution">
              <div class="pie-container">
                <svg
                  viewBox="0 0 320 320"
                  class="pie"
                  aria-label="Tool usage pie chart"
                >
                  <defs
                    ><filter
                      id="pie-shadow"
                      x="-25%"
                      y="-25%"
                      width="150%"
                      height="150%"
                      ><feDropShadow
                        dx="0"
                        dy="7"
                        stdDeviation="8"
                        flood-color="#7160ba"
                        flood-opacity="0.12"
                      /></filter
                    ></defs
                  >
                  <g filter="url(#pie-shadow)">
                    {#each chart as slice}
                      <path
                        d={slice.d}
                        fill={slice.color}
                        stroke="#fff"
                        stroke-width="3"
                        role="button"
                        tabindex="0"
                        aria-label={`${slice.name}: ${slice.count} executions (${slice.percent.toFixed(1)}%)`}
                        aria-pressed={selected === slice.name}
                        class:muted={selected && selected !== slice.name}
                        class:selected-slice={selected === slice.name}
                        style:--slice-offset={slice.translate}
                        onclick={() => choose(slice.name)}
                        onkeydown={(e) => {
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault();
                            choose(slice.name);
                          }
                        }}
                      >
                        <title
                          >{slice.name}: {slice.count} executions · {slice.percent.toFixed(
                            1,
                          )}%</title
                        >
                      </path>
                    {/each}
                  </g>
                </svg>
                <div class="chart-caption">
                  <span>↖</span> Select a slice to explore its executions
                </div>
              </div>
              <div class="legend">
                <div class="legend-head">
                  <span>TOOL</span><span>EXECUTIONS</span>
                </div>
                {#each chart as slice}<button
                    class:chosen={selected === slice.name}
                    class="legend-item"
                    onclick={() => choose(slice.name)}
                    aria-pressed={selected === slice.name}
                    ><span class="legend-name"
                      ><i style:background={slice.color}></i><strong
                        >{slice.name}</strong
                      ></span
                    ><span class="legend-value"
                      >{slice.count.toLocaleString()}<small
                        >{slice.percent.toFixed(1)}%</small
                      ></span
                    ></button
                  >{/each}
                {#if selected}<button
                    class="text-button clear-chart"
                    onclick={() => {
                      selected = '';
                      page = 0;
                    }}>Show all tools ↗</button
                  >{/if}
              </div>
            </div>
          {:else}
            <div class="chart-empty">
              <div class="empty-pie"></div>
              <h3>Your next command starts the story</h3>
              <p>
                {overview?.enabled
                  ? 'Run a SAT tool to see your usage take shape.'
                  : 'Enable history, then run a SAT tool to get started.'}
              </p>
            </div>
          {/if}
        </section>
        <section class="panel activity-panel" aria-labelledby="activity-title">
          <div class="panel-heading">
            <div>
              <h2 id="activity-title">The last 7 days</h2>
              <p>A little rhythm to your runtime.</p>
            </div>
            <span class="activity-symbol">⌁</span>
          </div>
          <div class="activity-number">
            {activity
              .reduce((sum, day) => sum + day.count, 0)
              .toLocaleString()}<span>executions this week</span>
          </div>
          <div class="bar-chart" aria-label="Daily execution counts in UTC">
            {#each activity as day}<div class="bar-column">
                <div class="bar-track">
                  <div
                    class="bar"
                    class:today={day === activity[6]}
                    style:height={`${Math.max(3, (day.count / maxActivity) * 100)}%`}
                    title={`${day.day}: ${day.count} executions`}
                  >
                    <span>{day.count}</span>
                  </div>
                </div>
                <span>{day.label}</span>
              </div>{/each}
          </div>
          <div class="activity-footer">
            <span class="subtle-dot purple"></span> Daily activity · UTC
          </div>
        </section>
      </div>

      <section
        id="executions"
        class="panel executions-panel"
        aria-labelledby="executions-title"
      >
        <div class="panel-heading execution-heading">
          <div>
            <h2 id="executions-title">
              {selected ? `${selected} executions` : 'Recent executions'}<span
                class="count-badge">{records.total.toLocaleString()}</span
              >
            </h2>
            <p>Every invocation, with the details that matter.</p>
          </div>
          {#if selected}<button
              class="filter-chip"
              onclick={() => {
                selected = '';
                page = 0;
              }}>{selected}<span>×</span></button
            >{/if}
        </div>
        <div class="filters">
          <label class="search-field"
            ><span>⌕</span><input
              aria-label="Search tools or arguments"
              placeholder="Search tools or arguments…"
              bind:value={search}
              oninput={() => (page = 0)}
            /></label
          >
          <div class="filter-selects">
            <label
              ><span class="sr-only">Tool</span><select
                aria-label="Filter by tool"
                bind:value={selected}
                onchange={() => (page = 0)}
                ><option value="">All tools</option
                >{#each overview?.tools ?? [] as tool}<option value={tool.name}
                    >{tool.name}</option
                  >{/each}</select
              ></label
            ><label
              ><span class="sr-only">Status</span><select
                aria-label="Filter by status"
                bind:value={status}
                onchange={() => (page = 0)}
                ><option value="">All statuses</option><option value="success"
                  >Successful</option
                ><option value="failed">Failed</option><option value="running"
                  >Unfinished</option
                ></select
              ></label
            >
          </div>
        </div>
        <div class="table-scroll">
          <table aria-busy={loading} class:empty={records.items.length === 0}>
            <thead
              ><tr
                ><th>TOOL</th><th>STARTED</th><th>STATUS</th><th>DURATION</th
                ><th>ARGUMENTS</th><th><span class="sr-only">Details</span></th
                ></tr
              ></thead
            ><tbody>
              {#each records.items as execution}<tr
                  ><td
                    ><button
                      class="tool-name"
                      onclick={() => choose(execution.tool)}
                      ><span
                        class="tool-symbol"
                        style:color={toolColor(execution.tool)}
                        style:background={`${toolColor(execution.tool)}16`}
                        >›_</span
                      >{execution.tool}</button
                    ></td
                  ><td class="time-cell"
                    ><time
                      datetime={new Date(execution.started_at_ms).toISOString()}
                      >{date(execution.started_at_ms)}</time
                    ></td
                  ><td
                    ><span class={`status-badge ${execution.status}`}
                      ><span
                        >{execution.status === 'success'
                          ? '✓'
                          : execution.status === 'failed'
                            ? '×'
                            : '◷'}</span
                      >{execution.status === 'success'
                        ? 'Success'
                        : execution.status === 'failed'
                          ? 'Failed'
                          : 'Unfinished'}</span
                    ></td
                  ><td class="duration-cell"
                    >{duration(execution.duration_ms)}</td
                  ><td class="args-cell"
                    ><button
                      onclick={() => (detail = execution)}
                      title="View all arguments"
                      ><code
                        >{execution.args.length
                          ? execution.args
                              .map((arg) => JSON.stringify(arg))
                              .join(' ')
                          : 'No arguments'}</code
                      ></button
                    ></td
                  ><td
                    ><button
                      class="row-detail"
                      aria-label={`View execution ${execution.id}`}
                      onclick={() => (detail = execution)}>↗</button
                    ></td
                  ></tr
                >
              {:else}<tr
                  ><td colspan="6"
                    ><div class="table-empty">
                      <span>⌘</span>
                      <h3>
                        {loading
                          ? 'Loading executions…'
                          : search || selected || status
                            ? 'No matching executions'
                            : 'A fresh start'}
                      </h3>
                      <p>
                        {search || selected || status
                          ? 'Try another tool, status, or search term.'
                          : 'Recorded tool invocations will appear here, along with their complete arguments.'}
                      </p>
                    </div></td
                  ></tr
                >{/each}
            </tbody>
          </table>
        </div>
        <div class="table-footer">
          <span
            >{records.total
              ? `${page * pageSize + 1}–${Math.min((page + 1) * pageSize, records.total)} of ${records.total}`
              : '0 executions'}<span class="footer-separator">·</span>Refreshes
            every 10 seconds</span
          >
          <div>
            <button
              class="pagination"
              aria-label="Previous page"
              disabled={page === 0 || loading}
              onclick={() => page--}>←</button
            ><span>Page {page + 1}</span><button
              class="pagination"
              aria-label="Next page"
              disabled={(page + 1) * pageSize >= records.total || loading}
              onclick={() => page++}>→</button
            >
          </div>
        </div>
      </section>
      <footer class="page-footer">
        <span
          ><span class="local-dot"></span> All data stays local. No telemetry is sent.</span
        ><button onclick={() => (help = true)}
          >Manage history <span>↗</span></button
        >
      </footer>
    </main>
  </div>
</div>

{#if detail || help}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(event) => {
      if (event.target === event.currentTarget) {
        detail = null;
        help = false;
      }
    }}
  >
    <div
      use:focusModal
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="dialog-title"
      tabindex="-1"
    >
      <div class="modal-heading">
        <div>
          <div class="eyebrow">
            {detail ? `EXECUTION #${detail.id}` : 'LOCAL HISTORY'}
          </div>
          <h2 id="dialog-title">
            {detail ? detail.tool : 'You’re in control.'}
          </h2>
        </div>
        <button
          class="icon-button"
          aria-label="Close details"
          onclick={() => {
            detail = null;
            help = false;
          }}>×</button
        >
      </div>
      {#if detail}
        <div class="detail-summary">
          <div>
            <small>Started</small><strong>{date(detail.started_at_ms)}</strong>
          </div>
          <div>
            <small>Duration</small><strong
              >{duration(detail.duration_ms)}</strong
            >
          </div>
          <div>
            <small>Result</small><strong
              class={`detail-status ${detail.status}`}
              >{detail.status}
              {detail.exit_code == null
                ? ''
                : `· exit ${detail.exit_code}`}</strong
            >
          </div>
        </div>
        <div class="detail-label">
          <h3>Command</h3>
          <button
            class="text-button"
            onclick={() =>
              copy(commandText(detail.tool, detail.args), 'command')}
            >{copied === 'command' ? 'Copied ✓' : 'Copy command ⧉'}</button
          >
        </div>
        <pre class="command-block">{commandText(detail.tool, detail.args)}</pre>
        <h3 class="arguments-heading">
          All provided arguments <span>{detail.args.length}</span>
        </h3>
        <ol class="argument-list">
          {#each detail.args as arg, i}<li>
              <span>{i + 1}</span>
              <pre>{JSON.stringify(arg)}</pre>
            </li>{:else}<li class="no-args">
              No arguments were provided.
            </li>{/each}
        </ol>
        {#if detail.error}<h3 class="arguments-heading">Error</h3>
          <pre class="error-block">{detail.error}</pre>{/if}
        {#if detail.status === 'running'}<p class="help-note">
            This invocation has not reported completion. It may still be
            running, or its process may have been interrupted.
          </p>{/if}
      {:else}
        <p class="modal-intro">
          Recording is opt-in. Your command history is stored on this machine
          and can be removed whenever you choose.
        </p>
        <div class="setting-command">
          <div>
            <strong>Enable recording</strong>
            <p>Save future tool invocations and complete arguments.</p>
          </div>
          <code>sat dashboard enable</code>
        </div>
        <div class="setting-command">
          <div>
            <strong>Pause recording</strong>
            <p>Keep your history, stop saving new executions.</p>
          </div>
          <code>sat dashboard disable</code>
        </div>
        <div class="setting-command">
          <div>
            <strong>Clear history</strong>
            <p>Delete the database. Recording preference stays the same.</p>
          </div>
          <code>sat dashboard reset</code>
        </div>
        <div class="storage-info">
          <span>LOCAL DATABASE</span><code
            >{overview?.database ?? 'SAT_HOME/executions.sqlite3'}</code
          >
        </div>
        <p class="help-note">
          Arguments can contain source code, paths, or secrets. Outputs and
          piped stdin are not recorded. No data is sent to an external service.
        </p>
      {/if}
    </div>
  </div>
{/if}
