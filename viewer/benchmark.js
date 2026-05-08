(function () {
  "use strict";

  const el = {
    benchRunId: document.getElementById("bench-run-id"),
    benchScenarioSeed: document.getElementById("bench-scenario-seed"),
    benchSuiteId: document.getElementById("bench-suite-id"),
    benchMetricsBody: document.getElementById("bench-metrics-body"),
    benchCaveat: document.getElementById("bench-caveat"),
    benchDetailsBody: document.getElementById("bench-details-body"),
    benchRunSelect: document.getElementById("bench-run-select"),
    benchRunInput: document.getElementById("bench-run-input"),
    benchRunLoad: document.getElementById("bench-run-load"),
    benchRunTrigger: document.getElementById("bench-run-trigger"),
    benchRunTriggerStatus: document.getElementById("bench-run-trigger-status"),
    benchGraphWrap: document.getElementById("bench-graph-wrap"),
    benchGraph: document.getElementById("bench-graph"),
  };

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function formatMetric(v) {
    if (typeof v !== "number" || Number.isNaN(v)) return "-";
    return Number.isInteger(v) ? String(v) : v.toFixed(4);
  }

  function renderBenchmarkPanel(payload) {
    if (!payload || !payload.run) {
      el.benchRunId.textContent = "not found";
      el.benchScenarioSeed.textContent = "-";
      el.benchSuiteId.textContent = "-";
      el.benchMetricsBody.innerHTML = '<tr><td colspan="3">No benchmark result found.</td></tr>';
      el.benchDetailsBody.innerHTML = '<tr><td colspan="5">No benchmark details found.</td></tr>';
      el.benchCaveat.textContent = "Run `benchmark-suite` first, then refresh this page.";
      return;
    }
    const run = payload.run;
    const byPolicy = new Map((payload.metrics || []).map((m) => [m.policy_variant, m]));
    const naive = byPolicy.get("naive_funded_by") || null;
    const conservative = byPolicy.get("conservative_funded_by") || null;
    const metricsOrder = [
      ["precision", "precision"],
      ["recall", "recall"],
      ["f1", "f1"],
      ["over_merge_rate", "over_merge_rate"],
      ["under_merge_rate", "under_merge_rate"],
      ["giant_component_inflation", "giant_component_inflation"],
      ["cluster_purity", "cluster_purity"],
      ["cluster_fragmentation", "cluster_fragmentation"],
    ];

    el.benchRunId.textContent = run.benchmark_run_id || "-";
    el.benchScenarioSeed.textContent = `${run.scenario_id || "-"} / ${run.seed ?? "-"}`;
    el.benchSuiteId.textContent = run.scenario_suite_id || "-";
    el.benchCaveat.textContent = payload.caveat || "Coordination evidence only.";
    el.benchMetricsBody.innerHTML = metricsOrder
      .map(([label, key]) => {
        const n = naive ? formatMetric(naive[key]) : "-";
        const c = conservative ? formatMetric(conservative[key]) : "-";
        return `<tr><td class="mono">${escapeHtml(label)}</td><td>${escapeHtml(n)}</td><td>${escapeHtml(c)}</td></tr>`;
      })
      .join("");

    const details = Array.isArray(payload.details) ? payload.details.slice(0, 60) : [];
    el.benchDetailsBody.innerHTML = details.length
      ? details
          .map(
            (d) => `<tr>
              <td>${escapeHtml(d.policy_variant || "-")}</td>
              <td class="mono">${escapeHtml(d.truth_entity_id || "-")}</td>
              <td>${escapeHtml(d.split_count ?? "-")}</td>
              <td>${escapeHtml(d.merge_intrusion_count ?? "-")}</td>
              <td>${escapeHtml(d.dominant_error_kind || "-")}</td>
            </tr>`
          )
          .join("")
      : '<tr><td colspan="5">No details found for this benchmark run.</td></tr>';
  }

  async function fetchPayload(runId) {
    const path = runId
      ? `/api/benchmark/run/${encodeURIComponent(runId)}`
      : "/api/benchmark/latest";
    const resp = await fetch(path, { cache: "no-store" });
    if (!resp.ok) return null;
    return resp.json();
  }

  async function loadAndRender(runId) {
    try {
      const payload = await fetchPayload(runId);
      renderBenchmarkPanel(payload);
      if (payload && payload.run && payload.run.benchmark_run_id) {
        await loadAndRenderGraph(payload.run.benchmark_run_id);
      }
    } catch (_err) {
      renderBenchmarkPanel(null);
    }
  }

  async function loadAndRenderGraph(runId) {
    if (!el.benchGraph || !el.benchGraphWrap || !window.d3) return;
    const resp = await fetch(`/api/benchmark/run/${encodeURIComponent(runId)}/graph`, { cache: "no-store" });
    if (!resp.ok) return;
    const graph = await resp.json();
    const d3 = window.d3;
    const width = el.benchGraphWrap.clientWidth || 900;
    const height = 520;
    const svg = d3.select(el.benchGraph);
    svg.selectAll("*").remove();
    svg.attr("viewBox", [0, 0, width, height]);

    const nodes = (graph.nodes || []).map((n) => ({ ...n }));
    const links = (graph.links || []).map((l) => ({ ...l }));
    const root = svg.append("g");
    svg.call(
      d3.zoom().scaleExtent([0.2, 5]).on("zoom", (event) => {
        root.attr("transform", event.transform);
      })
    );
    const linkSel = root
      .append("g")
      .selectAll("line")
      .data(links)
      .enter()
      .append("line")
      .attr("stroke", "#94a3b8")
      .attr("stroke-width", (l) => (l.strength === "strong" ? 2.8 : l.strength === "weak" ? 1.2 : 1.8))
      .attr("opacity", 0.7);
    const nodeSel = root
      .append("g")
      .selectAll("circle")
      .data(nodes)
      .enter()
      .append("circle")
      .attr("r", (n) => (n.kind === "identifier" ? 7 : 5))
      .attr("fill", (n) => (n.kind === "identifier" ? "#2563eb" : "#dc2626"))
      .attr("stroke", "#fff")
      .attr("stroke-width", 1.2);

    const sim = d3.forceSimulation(nodes)
      .force("link", d3.forceLink(links).id((n) => n.id).distance(45).strength(0.5))
      .force("charge", d3.forceManyBody().strength(-120))
      .force("center", d3.forceCenter(width / 2, height / 2))
      .force("collision", d3.forceCollide().radius((n) => (n.kind === "identifier" ? 9 : 7)));

    sim.on("tick", () => {
      linkSel
        .attr("x1", (l) => l.source.x)
        .attr("y1", (l) => l.source.y)
        .attr("x2", (l) => l.target.x)
        .attr("y2", (l) => l.target.y);
      nodeSel.attr("cx", (n) => n.x).attr("cy", (n) => n.y);
    });
  }

  async function loadRecentRuns() {
    try {
      const resp = await fetch("/api/benchmark/runs/recent", { cache: "no-store" });
      if (!resp.ok) return;
      const runs = await resp.json();
      if (!Array.isArray(runs) || !el.benchRunSelect) return;
      const opts = ['<option value="">latest</option>'];
      for (const r of runs) {
        const runId = escapeHtml(r.benchmark_run_id || "");
        const label = `${r.scenario_id || "-"} / seed=${r.seed ?? "-"} / ${r.scenario_suite_id || "-"}`;
        opts.push(`<option value="${runId}">${escapeHtml(label)}</option>`);
      }
      el.benchRunSelect.innerHTML = opts.join("");
    } catch (_err) {
      // keep default select only
    }
  }

  async function init() {
    await loadRecentRuns();
    await loadAndRender("");

    if (el.benchRunSelect) {
      el.benchRunSelect.addEventListener("change", async () => {
        const runId = (el.benchRunSelect.value || "").trim();
        await loadAndRender(runId);
      });
    }
    if (el.benchRunLoad) {
      el.benchRunLoad.addEventListener("click", async () => {
        const runId = (el.benchRunInput && el.benchRunInput.value || "").trim();
        if (!runId) {
          await loadAndRender("");
          return;
        }
        await loadAndRender(runId);
      });
    }
    if (el.benchRunTrigger) {
      el.benchRunTrigger.addEventListener("click", async () => {
        try {
          el.benchRunTrigger.disabled = true;
          if (el.benchRunTriggerStatus) {
            el.benchRunTriggerStatus.textContent = "Running simulation...";
          }
          const resp = await fetch("/api/benchmark/run-simulation", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({}),
          });
          if (!resp.ok) {
            if (el.benchRunTriggerStatus) {
              el.benchRunTriggerStatus.textContent = "Simulation failed. Check server logs.";
            }
            return;
          }
          const payload = await resp.json();
          renderBenchmarkPanel(payload);
          await loadRecentRuns();
          if (el.benchRunTriggerStatus) {
            el.benchRunTriggerStatus.textContent = "Simulation complete. Loaded latest result.";
          }
        } catch (_err) {
          if (el.benchRunTriggerStatus) {
            el.benchRunTriggerStatus.textContent = "Simulation failed. Check server logs.";
          }
        } finally {
          el.benchRunTrigger.disabled = false;
        }
      });
    }
  }

  init();
})();
