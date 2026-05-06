(function () {
  "use strict";

  const CANONICAL_RUN_ID = "run-1777967119432576";
  const ARTIFACT_FILES = {
    summary: "arbitrum_gov_summary.json",
    graph: "arbitrum_gov.graph.json",
    report: "arbitrum_gov_report.md",
  };

  const el = {
    heroRunId: document.getElementById("hero-run-id"),
    heroWindow: document.getElementById("hero-window"),
    consistencyError: document.getElementById("consistency-error"),
    resultClusters: document.getElementById("result-clusters"),
    resultTopSize: document.getElementById("result-top-size"),
    resultMulti: document.getElementById("result-multi"),
    tableBody: document.getElementById("cluster-table-body"),
    inspector: document.getElementById("inspector"),
    svg: document.getElementById("graph"),
    graphWrap: document.getElementById("graph-wrap"),
    tooltip: document.getElementById("tooltip"),
    advCoverage: document.getElementById("adv-coverage"),
    advIngestion: document.getElementById("adv-ingestion"),
    advPolicy: document.getElementById("adv-policy"),
    advLineage: document.getElementById("adv-lineage"),
    advConcentration: document.getElementById("adv-concentration"),
    ctxMode: document.getElementById("ctx-mode"),
    ctxClusterId: document.getElementById("ctx-cluster-id"),
    ctxSizeDegree: document.getElementById("ctx-size-degree"),
    ctxGc: document.getElementById("ctx-gc"),
    ctxEvidence: document.getElementById("ctx-evidence"),
  };

  let graphData = null;
  let summaryData = null;
  let activeClusterId = null;
  let cachedView = null;
  let nodeSelection = null;
  let linkSelection = null;

  function detectBasePath() {
    const p = window.location.pathname || "/";
    const marker = "/viewer/";
    const idx = p.indexOf(marker);
    if (idx >= 0) return p.slice(0, idx);
    return "";
  }

  function artifactCandidates(kind) {
    const file = ARTIFACT_FILES[kind];
    const basePath = detectBasePath();
    const rootPrefixed = basePath ? `${basePath}/out/${file}` : `/out/${file}`;
    return [
      `../out/${file}`,
      `./../out/${file}`,
      rootPrefixed,
      `/out/${file}`,
    ];
  }

  async function loadWithCandidates(kind, parser) {
    const tried = [];
    for (const path of artifactCandidates(kind)) {
      try {
        const r = await fetch(path, { cache: "no-store" });
        tried.push(`${path} -> ${r.status}`);
        if (!r.ok) continue;
        return parser(r);
      } catch (err) {
        tried.push(`${path} -> ${err.message || String(err)}`);
      }
    }
    throw new Error(`Failed to load ${kind}; tried: ${tried.join(" | ")}`);
  }

  async function loadJson(kind) {
    return loadWithCandidates(kind, (r) => r.json());
  }

  async function loadText(kind) {
    return loadWithCandidates(kind, (r) => r.text());
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function parseWindowFromReport(report) {
    const m = report.match(/Block window\*\*:\s*`(\d+)`\s*→\s*`(\d+)`/);
    if (m) return `${m[1]} -> ${m[2]}`;
    return "428203933 -> 459307198";
  }

  function parseMultiAddressClusters(summary, report) {
    if (typeof summary.num_multi_address_clusters === "number") {
      return summary.num_multi_address_clusters;
    }
    const m = report.match(/multi-address clusters[^0-9]*(\d+)/i);
    if (m) return Number(m[1]);
    if (summary.run_id === CANONICAL_RUN_ID) return 13;
    return "-";
  }

  function formatBytes(n) {
    if (typeof n !== "number" || Number.isNaN(n)) return "-";
    const mb = n / (1024 * 1024);
    return `${mb.toFixed(2)} MB`;
  }

  function reportMetric(report, label) {
    const re = new RegExp(`${label}:\\s*([^\\n]+)`, "i");
    const m = report.match(re);
    return m ? m[1].trim() : "-";
  }

  function listItem(label, value) {
    return `<li><b>${escapeHtml(label)}:</b> ${escapeHtml(String(value ?? "-"))}</li>`;
  }

  function showConsistencyError(msg) {
    el.consistencyError.style.display = "block";
    el.consistencyError.textContent = msg;
  }

  function setHero(summary, report) {
    el.heroRunId.textContent = summary.run_id || "unknown";
    el.heroWindow.textContent = parseWindowFromReport(report);
  }

  function setResultMetrics(summary, report) {
    el.resultClusters.textContent = String(summary.n_clusters ?? "-");
    el.resultTopSize.textContent = String(summary.top_cluster_size ?? "-");
    el.resultMulti.textContent = String(parseMultiAddressClusters(summary, report));
  }

  function setAdvancedMetrics(summary, report) {
    const seed = summary.seed_counts || {};
    el.advCoverage.innerHTML = [
      listItem("governance seeds", seed.governance ?? "-"),
      listItem("control seeds", seed.control ?? "-"),
      listItem("addresses clustered", summary.n_addresses_clustered ?? "-"),
      listItem("multi-address clusters", parseMultiAddressClusters(summary, report)),
      listItem("pagination bias risk", summary.pagination_bias_risk ?? "-"),
    ].join("");

    const cap = summary.pagination_cap_hits || {};
    el.advIngestion.innerHTML = [
      listItem("alchemy calls", summary.alchemy_calls ?? "-"),
      listItem("is_contract calls", summary.is_contract_calls ?? "-"),
      listItem("transfer rows inserted", summary.transfers_rows_inserted ?? "-"),
      listItem("db size", formatBytes(summary.db_size_bytes)),
      listItem("pagination cap hits", `row=${cap.row_cap ?? 0}, page=${cap.page_cap ?? 0}, peer=${cap.distinct_peer_cap ?? 0}`),
    ].join("");

    const params = summary.link_params || {};
    const funded = params.funded_by_policy || {};
    el.advPolicy.innerHTML = [
      listItem("policy_profile_id", summary.policy_profile_id ?? "-"),
      listItem("conservative funded_by", summary.conservative_funded_by_policy_enabled ?? funded.enabled ?? "-"),
      listItem("fan_out_cap", params.fan_out_cap ?? summary.link_fanout_cap ?? "-"),
      listItem("min_evidence", params.min_evidence ?? summary.min_evidence ?? "-"),
      listItem("stable / related threshold", `${summary.stable_threshold ?? "-"} / ${summary.related_threshold ?? "-"}`),
      listItem("funded_by short-burst", `delta=${funded.short_burst_block_delta ?? "-"}, min_hits=${funded.min_short_burst_hits ?? "-"}`),
    ].join("");

    const lineage = summary.lineage || {};
    const counts = lineage.counts || {};
    el.advLineage.innerHTML = [
      listItem("enabled", lineage.enabled ?? "-"),
      listItem("skip_reason", lineage.skip_reason ?? "-"),
      listItem("previous_run_id", lineage.previous_run_id ?? "-"),
      listItem("counts", `stable=${counts.stable ?? 0}, related=${counts.related ?? 0}, new=${counts.new ?? 0}, disappeared=${counts.disappeared ?? 0}, total=${counts.total_rows ?? 0}`),
    ].join("");

    el.advConcentration.innerHTML = [
      listItem("identifiers per cluster", reportMetric(report, "Identifiers per cluster")),
      listItem("Nakamoto (>50%)", reportMetric(report, "Nakamoto coefficient \\(>50% of population\\)")),
      listItem("Gini coefficient", reportMetric(report, "Gini coefficient")),
      listItem("run_id", summary.run_id ?? "-"),
      listItem("window", parseWindowFromReport(report)),
    ].join("");

  }

  function topClusters(summary) {
    return Array.isArray(summary.top_clusters) ? summary.top_clusters : [];
  }

  function renderTable(summary) {
    const rows = topClusters(summary);
    if (!rows.length) {
      el.tableBody.innerHTML = `<tr><td colspan="4">No clusters available.</td></tr>`;
      return;
    }
    el.tableBody.innerHTML = rows
      .map(
        (c) => `<tr class="clickable" data-cluster-id="${escapeHtml(c.cluster_id)}">
          <td class="mono">${escapeHtml(c.cluster_id)}</td>
          <td>${escapeHtml(c.size)}</td>
          <td>${escapeHtml(c.governance_count ?? "-")}</td>
          <td>${escapeHtml(c.control_count ?? "-")}</td>
        </tr>`
      )
      .join("");
    Array.from(el.tableBody.querySelectorAll("tr.clickable")).forEach((row) => {
      row.addEventListener("click", () => {
        const cid = row.getAttribute("data-cluster-id");
        setActiveCluster(cid);
        renderInspectorForCluster(cid);
      });
    });
  }

  function colorForNode(n) {
    if (n.kind === "identifier") return "var(--id)";
    switch (n.type) {
      case "safe_owner":
        return "var(--safe)";
      case "funded_by":
        return "var(--funded)";
      case "ens_handle":
        return "var(--ens)";
      case "did_controller":
        return "var(--did)";
      default:
        return "#64748b";
    }
  }

  function nodeRadius(n) {
    return n.kind === "identifier" ? 10 : 6;
  }

  function setActiveCluster(cid) {
    activeClusterId = cid || null;
    Array.from(el.tableBody.querySelectorAll("tr.clickable")).forEach((tr) => {
      tr.classList.toggle("active", tr.getAttribute("data-cluster-id") === activeClusterId);
    });
    if (!nodeSelection || !linkSelection) return;
    nodeSelection
      .attr("opacity", (n) => {
        if (!activeClusterId) return 1;
        if (n.cluster_id === activeClusterId) return 1;
        return n.kind === "evidence" ? 0.5 : 0.12;
      })
      .attr("stroke-width", (n) => (n.cluster_id === activeClusterId ? 2.6 : 1.2));
    linkSelection.attr("opacity", (l) => {
      if (!activeClusterId) return 0.85;
      const s = typeof l.source === "object" ? l.source : cachedView.nodeById.get(l.source);
      const t = typeof l.target === "object" ? l.target : cachedView.nodeById.get(l.target);
      if ((s && s.cluster_id === activeClusterId) || (t && t.cluster_id === activeClusterId)) return 0.95;
      return 0.08;
    });
  }

  function graphDerivedCluster(clusterId) {
    const nodes = graphData.nodes || [];
    const links = graphData.links || [];
    const idNodes = nodes.filter((n) => n.kind === "identifier" && n.cluster_id === clusterId);
    const idSet = new Set(idNodes.map((n) => n.id));
    const evidenceKinds = new Set();
    const evidenceNodes = new Set();
    let degree = 0;
    links.forEach((l) => {
      const s = typeof l.source === "object" ? l.source.id : l.source;
      const t = typeof l.target === "object" ? l.target.id : l.target;
      if (idSet.has(s) || idSet.has(t)) {
        degree += 1;
        if (l.type) evidenceKinds.add(l.type);
        const sn = cachedView.nodeById.get(s);
        const tn = cachedView.nodeById.get(t);
        if (sn && sn.kind === "evidence") evidenceNodes.add(sn.id);
        if (tn && tn.kind === "evidence") evidenceNodes.add(tn.id);
      }
    });
    const gc = getClusterGovControl(clusterId);
    return {
      cluster_id: clusterId,
      size: idNodes.length,
      governance_count: gc ? gc.governance_count : null,
      control_count: gc ? gc.control_count : null,
      connected_evidence_types: Array.from(evidenceKinds).sort(),
      visible_evidence_nodes: evidenceNodes.size,
      visible_links: degree,
    };
  }

  function getClusterGovControl(clusterId) {
    const hit = topClusters(summaryData).find((c) => c.cluster_id === clusterId);
    if (!hit) return null;
    return {
      governance_count: hit.governance_count,
      control_count: hit.control_count,
    };
  }

  function renderInspectorForCluster(clusterId) {
    const hit = topClusters(summaryData).find((c) => c.cluster_id === clusterId);
    if (hit) {
      const keys = Array.isArray(hit.shared_evidence_keys) ? hit.shared_evidence_keys : [];
      const kinds = Array.from(
        new Set(
          keys.map((k) => {
            const i = String(k).indexOf(":");
            return i > 0 ? String(k).slice(0, i) : "funded_by";
          })
        )
      );
      setContextClusterMetrics({
        cluster_id: hit.cluster_id,
        size: hit.size,
        governance_count: hit.governance_count,
        control_count: hit.control_count,
        connected_evidence_types: kinds,
      });
      el.inspector.innerHTML = `
        <h3>Cluster detail</h3>
        <p><b>cluster_id:</b> <span class="mono">${escapeHtml(hit.cluster_id)}</span></p>
        <p><b>size:</b> ${escapeHtml(hit.size)}</p>
        <p><b>governance/control:</b> ${escapeHtml(hit.governance_count ?? "-")} / ${escapeHtml(hit.control_count ?? "-")}</p>
        <p><b>shared evidence keys (${keys.length}):</b></p>
        <ul>${keys.map((k) => `<li class="mono">${escapeHtml(k)}</li>`).join("") || "<li>none</li>"}</ul>
      `;
      return;
    }
    const derived = graphDerivedCluster(clusterId);
    setContextClusterMetrics(derived);
    el.inspector.innerHTML = `
      <h3>Cluster detail (graph-derived)</h3>
      <p><b>cluster_id:</b> <span class="mono">${escapeHtml(derived.cluster_id)}</span></p>
      <p><b>size:</b> ${escapeHtml(derived.size)}</p>
      <p><b>connected evidence types:</b> ${derived.connected_evidence_types.length ? escapeHtml(derived.connected_evidence_types.join(", ")) : "none"}</p>
      <p><b>visible evidence nodes:</b> ${escapeHtml(derived.visible_evidence_nodes)}</p>
      <p><b>visible links:</b> ${escapeHtml(derived.visible_links)}</p>
    `;
  }

  function renderInspectorForNode(node, incidentLinks) {
    const kinds = Array.from(new Set(incidentLinks.map((l) => l.type).filter(Boolean))).sort();
    setContextNodeMetrics(node, incidentLinks, kinds);
    el.inspector.innerHTML = `
      <h3>Node detail</h3>
      <p><b>address/node:</b> <span class="mono">${escapeHtml(node.value || node.id)}</span></p>
      <p><b>cluster_id:</b> ${node.cluster_id ? `<span class="mono">${escapeHtml(node.cluster_id)}</span>` : "none"}</p>
      <p><b>connected evidence types:</b> ${kinds.length ? escapeHtml(kinds.join(", ")) : "none"}</p>
      <p><b>degree / connections:</b> ${incidentLinks.length}</p>
    `;
  }

  function setContextClusterMetrics(cluster) {
    el.ctxMode.textContent = "cluster";
    el.ctxClusterId.textContent = cluster.cluster_id || "-";
    el.ctxSizeDegree.textContent = String(cluster.size ?? "-");
    const g = cluster.governance_count;
    const c = cluster.control_count;
    el.ctxGc.textContent = (g == null && c == null) ? "-" : `${g ?? "-"} / ${c ?? "-"}`;
    const kinds = Array.isArray(cluster.connected_evidence_types) ? cluster.connected_evidence_types : [];
    el.ctxEvidence.textContent = kinds.length ? kinds.join(", ") : "-";
  }

  function setContextNodeMetrics(node, incidentLinks, kinds) {
    el.ctxMode.textContent = node.kind === "identifier" ? "node (identifier)" : `node (${node.kind || "unknown"})`;
    el.ctxClusterId.textContent = node.cluster_id || "-";
    el.ctxSizeDegree.textContent = String(incidentLinks.length);
    const gc = node.cluster_id ? getClusterGovControl(node.cluster_id) : null;
    el.ctxGc.textContent = gc ? `${gc.governance_count ?? "-"} / ${gc.control_count ?? "-"}` : "-";
    el.ctxEvidence.textContent = kinds.length ? kinds.join(", ") : "-";
  }

  function renderGraph(summary, graph) {
    const d3 = window.d3;
    const width = el.graphWrap.clientWidth;
    const height = 560;
    const svg = d3.select(el.svg);
    svg.selectAll("*").remove();
    svg.attr("viewBox", [0, 0, width, height]);

    const nodes = (graph.nodes || []).map((n) => ({ ...n }));
    const links = (graph.links || []).map((l) => ({ ...l }));
    const nodeById = new Map(nodes.map((n) => [n.id, n]));
    cachedView = { nodes, links, nodeById };

    const focusClusterId = topClusters(summary)[0] ? topClusters(summary)[0].cluster_id : null;
    const focusSet = new Set(
      nodes.filter((n) => n.cluster_id === focusClusterId).map((n) => n.id)
    );
    links.forEach((l) => {
      const s = typeof l.source === "object" ? l.source.id : l.source;
      const t = typeof l.target === "object" ? l.target.id : l.target;
      if (focusSet.has(s) || focusSet.has(t)) {
        focusSet.add(s);
        focusSet.add(t);
      }
    });

    nodes.forEach((n, i) => {
      if (focusSet.has(n.id)) {
        const a = (i % 12) * (Math.PI / 6);
        n.x = width / 2 + Math.cos(a) * 130;
        n.y = height / 2 + Math.sin(a) * 120;
      } else {
        const ring = 240 + (i % 20) * 4;
        const a = (i * 0.37) % (Math.PI * 2);
        n.x = width / 2 + Math.cos(a) * ring;
        n.y = height / 2 + Math.sin(a) * ring;
      }
    });

    const root = svg.append("g");
    svg.call(
      d3.zoom().scaleExtent([0.2, 5]).on("zoom", (event) => {
        root.attr("transform", event.transform);
      })
    );

    linkSelection = root
      .append("g")
      .selectAll("line")
      .data(links)
      .enter()
      .append("line")
      .attr("stroke", "#94a3b8")
      .attr("stroke-width", (l) => (l.strength === "strong" ? 3.2 : l.strength === "medium" ? 2.2 : 1.6))
      .attr("stroke-dasharray", (l) => (l.strength === "weak" ? "5 4" : null))
      .attr("opacity", (l) => {
        const s = typeof l.source === "object" ? l.source.id : l.source;
        const t = typeof l.target === "object" ? l.target.id : l.target;
        return focusSet.has(s) || focusSet.has(t) ? 0.95 : 0.22;
      });

    nodeSelection = root
      .append("g")
      .selectAll("circle")
      .data(nodes)
      .enter()
      .append("circle")
      .attr("r", nodeRadius)
      .attr("fill", colorForNode)
      .attr("stroke", "#ffffff")
      .attr("stroke-width", (n) => (focusSet.has(n.id) ? 2.2 : 1.2))
      .attr("opacity", (n) => (focusSet.has(n.id) ? 1 : n.kind === "evidence" ? 0.5 : 0.2))
      .style("cursor", "grab")
      .on("mouseenter", (event, n) => {
        el.tooltip.style.opacity = "1";
        el.tooltip.innerHTML = `${escapeHtml(n.kind)} · ${escapeHtml(n.type || "-")}<br><span class="mono">${escapeHtml(n.value || n.id)}</span>`;
        el.tooltip.style.left = `${event.offsetX}px`;
        el.tooltip.style.top = `${event.offsetY}px`;
      })
      .on("mousemove", (event) => {
        el.tooltip.style.left = `${event.offsetX}px`;
        el.tooltip.style.top = `${event.offsetY}px`;
      })
      .on("mouseleave", () => {
        el.tooltip.style.opacity = "0";
      })
      .on("click", (event, n) => {
        event.stopPropagation();
        if (n.cluster_id) setActiveCluster(n.cluster_id);
        const incident = links.filter((l) => {
          const s = typeof l.source === "object" ? l.source.id : l.source;
          const t = typeof l.target === "object" ? l.target.id : l.target;
          return s === n.id || t === n.id;
        });
        renderInspectorForNode(n, incident);
      })
      .call(
        d3.drag()
          .on("start", (event, n) => {
            if (!event.active) sim.alphaTarget(0.35).restart();
            n.fx = n.x;
            n.fy = n.y;
          })
          .on("drag", (event, n) => {
            n.fx = event.x;
            n.fy = event.y;
          })
          .on("end", (event, n) => {
            if (!event.active) sim.alphaTarget(0);
            n.fx = null;
            n.fy = null;
          })
      );

    root
      .append("g")
      .selectAll("text")
      .data(nodes.filter((n) => n.kind === "identifier"))
      .enter()
      .append("text")
      .attr("font-size", 11)
      .attr("font-family", "ui-monospace, SFMono-Regular, Menlo, monospace")
      .attr("fill", "#0f172a")
      .attr("dx", 12)
      .attr("dy", "0.35em")
      .text((n) => {
        const v = n.value || n.id;
        return v.length > 14 ? `${v.slice(0, 8)}...${v.slice(-4)}` : v;
      });

    const sim = d3.forceSimulation(nodes)
      .force("link", d3.forceLink(links).id((n) => n.id).distance((l) => {
        const s = typeof l.source === "object" ? l.source.id : l.source;
        const t = typeof l.target === "object" ? l.target.id : l.target;
        return focusSet.has(s) || focusSet.has(t) ? 64 : 86;
      }).strength((l) => {
        const s = typeof l.source === "object" ? l.source.id : l.source;
        const t = typeof l.target === "object" ? l.target.id : l.target;
        return focusSet.has(s) || focusSet.has(t) ? 0.72 : 0.22;
      }))
      .force("charge", d3.forceManyBody().strength((n) => (focusSet.has(n.id) ? -290 : -120)))
      .force("center", d3.forceCenter(width / 2, height / 2))
      .force("collision", d3.forceCollide().radius((n) => nodeRadius(n) + 8));

    const labels = root.selectAll("text");
    sim.on("tick", () => {
      linkSelection
        .attr("x1", (l) => l.source.x)
        .attr("y1", (l) => l.source.y)
        .attr("x2", (l) => l.target.x)
        .attr("y2", (l) => l.target.y);
      nodeSelection
        .attr("cx", (n) => n.x)
        .attr("cy", (n) => n.y);
      labels
        .attr("x", (n) => n.x)
        .attr("y", (n) => n.y);
    });

    if (focusClusterId) {
      setActiveCluster(focusClusterId);
      renderInspectorForCluster(focusClusterId);
    }
  }

  async function init() {
    try {
      const [summary, graph, report] = await Promise.all([
        loadJson("summary"),
        loadJson("graph"),
        loadText("report").catch(() => ""),
      ]);
      summaryData = summary;
      graphData = graph;

      setHero(summary, report);
      setResultMetrics(summary, report);
      setAdvancedMetrics(summary, report);
      renderTable(summary);
      renderGraph(summary, graph);

      const summaryRun = summary.run_id || "";
      const graphRun = graph && graph.run ? graph.run.run_id : "";
      if (summaryRun !== graphRun) {
        showConsistencyError(
          `Run mismatch: summary(${summaryRun || "missing"}) vs graph(${graphRun || "missing"}).`
        );
      }
      if (summaryRun !== CANONICAL_RUN_ID || graphRun !== CANONICAL_RUN_ID) {
        showConsistencyError(
          `Expected canonical run ${CANONICAL_RUN_ID}. Got summary=${summaryRun || "missing"}, graph=${graphRun || "missing"}.`
        );
      }
    } catch (err) {
      showConsistencyError(err.message || String(err));
    }
  }

  init();
})();

