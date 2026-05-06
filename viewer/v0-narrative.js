(function () {
  "use strict";

  const CANONICAL_RUN_ID = "run-1777967119432576";
  const PATHS = {
    summary: "../out/arbitrum_gov_summary.json",
    graph: "../out/arbitrum_gov.graph.json",
    report: "../out/arbitrum_gov_report.md",
  };

  const el = {
    runBadge: document.getElementById("run-badge"),
    profileBadge: document.getElementById("profile-badge"),
    consistencyBadge: document.getElementById("consistency-badge"),
    consistencyErr: document.getElementById("consistency-error"),
    nClusters: document.getElementById("n-clusters"),
    topSize: document.getElementById("top-size"),
    policyProfile: document.getElementById("policy-profile"),
    tableBody: document.getElementById("cluster-table-body"),
    inspector: document.getElementById("inspector"),
    caveats: document.getElementById("caveats-list"),
    svg: document.getElementById("graph"),
    graphWrap: document.getElementById("graph-wrap"),
    tooltip: document.getElementById("tooltip"),
  };

  function setConsistency(ok, text, err) {
    el.consistencyBadge.textContent = text;
    el.consistencyBadge.style.background = ok ? "#ecfdf5" : "#fef2f2";
    el.consistencyBadge.style.color = ok ? "#166534" : "#991b1b";
    el.consistencyBadge.style.borderColor = ok ? "#86efac" : "#fecaca";
    if (err) {
      el.consistencyErr.textContent = err;
      el.consistencyErr.style.display = "block";
    } else {
      el.consistencyErr.textContent = "";
      el.consistencyErr.style.display = "none";
    }
  }

  async function loadJson(path) {
    const r = await fetch(path, { cache: "no-store" });
    if (!r.ok) {
      throw new Error(`${path}: ${r.status} ${r.statusText}`);
    }
    return r.json();
  }

  async function loadText(path) {
    const r = await fetch(path, { cache: "no-store" });
    if (!r.ok) {
      throw new Error(`${path}: ${r.status} ${r.statusText}`);
    }
    return r.text();
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function renderCaveats(reportMd) {
    const fixed = [
      "This is not Sybil detection.",
      "This is not real-world identity attribution.",
      "Clusters are coordination-evidence structures, not claims about humans or malicious actors.",
    ];
    const optional = [];
    const lines = reportMd.split("\n").map((x) => x.trim());
    const extraNeedles = [
      "Governance participation is not malicious behavior.",
      "Shared funder or sink activity does not imply the same human.",
    ];
    for (const needle of extraNeedles) {
      const line = lines.find((l) => l.includes(needle));
      if (line) optional.push(line.replace(/^-+\s*/, ""));
    }
    el.caveats.innerHTML = [...fixed, ...optional]
      .map((m) => `<li>${escapeHtml(m)}</li>`)
      .join("");
  }

  function renderSummary(summary) {
    el.runBadge.textContent = `run: ${summary.run_id || "unknown"}`;
    el.profileBadge.textContent = `profile: ${summary.policy_profile_id || "unknown"}`;
    el.nClusters.textContent = String(summary.n_clusters ?? "-");
    el.topSize.textContent = String(summary.top_cluster_size ?? "-");
    el.policyProfile.textContent = summary.policy_profile_id || "-";
  }

  function renderTable(summary, onSelectCluster) {
    const rows = Array.isArray(summary.top_clusters) ? summary.top_clusters.slice(0, 10) : [];
    if (!rows.length) {
      el.tableBody.innerHTML = `<tr><td colspan="5" class="muted">No top_clusters available</td></tr>`;
      return;
    }
    el.tableBody.innerHTML = rows
      .map(
        (c) => `<tr class="clickable" data-cluster="${escapeHtml(c.cluster_id)}">
          <td class="mono">${escapeHtml(c.cluster_id)}</td>
          <td>${escapeHtml(c.size)}</td>
          <td>${escapeHtml(c.coordination_tier || "-")}</td>
          <td>${escapeHtml(c.governance_count ?? "-")}</td>
          <td>${escapeHtml(c.control_count ?? "-")}</td>
        </tr>`
      )
      .join("");
    Array.from(el.tableBody.querySelectorAll("tr.clickable")).forEach((tr) => {
      tr.addEventListener("click", () => onSelectCluster(tr.getAttribute("data-cluster")));
    });
  }

  function makeInspectorHtml(cluster) {
    if (!cluster) {
      return `<p class="muted small">No cluster detail available.</p>`;
    }
    const keys = Array.isArray(cluster.shared_evidence_keys) ? cluster.shared_evidence_keys : [];
    return `
      <h3>Cluster detail</h3>
      <p><span class="mono">${escapeHtml(cluster.cluster_id)}</span></p>
      <p class="small"><b>size:</b> ${escapeHtml(cluster.size)} · <b>tier:</b> ${escapeHtml(cluster.coordination_tier || "-")}</p>
      <p class="small"><b>gov/control:</b> ${escapeHtml(cluster.governance_count ?? "-")} / ${escapeHtml(cluster.control_count ?? "-")}</p>
      <p class="small"><b>shared_evidence_keys (${keys.length}):</b></p>
      <ul class="small">${keys.map((k) => `<li class="mono">${escapeHtml(k)}</li>`).join("") || "<li>none</li>"}</ul>
    `;
  }

  function summarizeGraphCluster(node, graph) {
    const nodes = graph.nodes || [];
    const links = graph.links || [];
    const clusterId = node.cluster_id || null;
    const idSet = new Set();
    const evidenceSet = new Set();
    const connectedKinds = new Set();

    if (clusterId) {
      nodes.forEach((n) => {
        if (n.cluster_id === clusterId) {
          if (n.kind === "identifier") idSet.add(n.id);
          if (n.kind === "evidence") evidenceSet.add(n.id);
        }
      });
    }
    if (idSet.size === 0 && node.kind === "identifier") {
      idSet.add(node.id);
    }

    let connectedEdgeCount = 0;
    links.forEach((l) => {
      const s = typeof l.source === "object" ? l.source.id : l.source;
      const t = typeof l.target === "object" ? l.target.id : l.target;
      if (idSet.has(s) || idSet.has(t) || s === node.id || t === node.id) {
        connectedEdgeCount += 1;
        if (l.type) connectedKinds.add(l.type);
        const srcNode = nodes.find((n) => n.id === s);
        const dstNode = nodes.find((n) => n.id === t);
        if (srcNode && srcNode.kind === "evidence") evidenceSet.add(srcNode.id);
        if (dstNode && dstNode.kind === "evidence") evidenceSet.add(dstNode.id);
      }
    });

    return {
      nodeId: node.id,
      clusterId,
      identifierCount: idSet.size,
      evidenceNodeCount: evidenceSet.size,
      connectedEdgeCount,
      connectedEvidenceKinds: Array.from(connectedKinds).sort(),
      identifiers: Array.from(idSet),
      evidenceNodes: Array.from(evidenceSet),
    };
  }

  function makeGraphFallbackInspector(node, graph) {
    const info = summarizeGraphCluster(node, graph);
    return `
      <h3>Graph-derived cluster context</h3>
      <p class="small"><b>node_id:</b> <span class="mono">${escapeHtml(info.nodeId)}</span></p>
      <p class="small"><b>cluster_id:</b> ${info.clusterId ? `<span class="mono">${escapeHtml(info.clusterId)}</span>` : "none in graph node"}</p>
      <p class="small"><b>visible identifiers:</b> ${info.identifierCount}</p>
      <p class="small"><b>visible evidence nodes:</b> ${info.evidenceNodeCount}</p>
      <p class="small"><b>connected evidence links:</b> ${info.connectedEdgeCount}</p>
      <p class="small"><b>evidence kinds:</b> ${info.connectedEvidenceKinds.length ? escapeHtml(info.connectedEvidenceKinds.join(", ")) : "none"}</p>
      <details class="small"><summary>identifier nodes in view</summary><ul>${info.identifiers.map((x) => `<li class="mono">${escapeHtml(x)}</li>`).join("") || "<li>none</li>"}</ul></details>
      <details class="small"><summary>evidence nodes in view</summary><ul>${info.evidenceNodes.map((x) => `<li class="mono">${escapeHtml(x)}</li>`).join("") || "<li>none</li>"}</ul></details>
    `;
  }

  function colorForNode(d) {
    if (d.kind === "identifier") return "var(--id)";
    switch (d.type) {
      case "safe_owner":
        return "var(--safe)";
      case "funded_by":
        return "var(--funded)";
      case "ens_handle":
        return "var(--ens)";
      case "did_controller":
        return "var(--did)";
      default:
        return "#6b7280";
    }
  }

  function nodeRadius(d) {
    return d.kind === "identifier" ? 8 : 5;
  }

  function renderGraph(graph, summary) {
    if (!window.d3) {
      setConsistency(
        false,
        "consistency: blocked",
        "D3 runtime not found at viewer/vendor/d3.v7.min.js. Add local D3 to run this static viewer offline."
      );
      return;
    }
    const d3 = window.d3;
    const width = el.graphWrap.clientWidth;
    const height = 500;

    const svg = d3.select(el.svg);
    svg.selectAll("*").remove();
    svg.attr("viewBox", [0, 0, width, height]);

    const root = svg.append("g");
    const zoom = d3.zoom().scaleExtent([0.2, 4]).on("zoom", (event) => {
      root.attr("transform", event.transform);
    });
    svg.call(zoom);

    const nodes = (graph.nodes || []).map((x) => ({ ...x }));
    const links = (graph.links || []).map((x) => ({ ...x }));

    const sim = d3
      .forceSimulation(nodes)
      .force("link", d3.forceLink(links).id((d) => d.id).distance(72).strength(0.5))
      .force("charge", d3.forceManyBody().strength(-220))
      .force("center", d3.forceCenter(width / 2, height / 2))
      .force("collision", d3.forceCollide().radius((d) => nodeRadius(d) + 6));

    const linkVis = root
      .append("g")
      .selectAll("line")
      .data(links)
      .enter()
      .append("line")
      .attr("stroke", "#94a3b8")
      .attr("stroke-width", (d) => (d.strength === "strong" ? 2.2 : d.strength === "medium" ? 1.4 : 1.0))
      .attr("stroke-dasharray", (d) => (d.strength === "weak" ? "4 3" : null))
      .attr("opacity", 0.85);

    const nodeVis = root
      .append("g")
      .selectAll("circle")
      .data(nodes)
      .enter()
      .append("circle")
      .attr("r", nodeRadius)
      .attr("fill", colorForNode)
      .attr("stroke", "#fff")
      .attr("stroke-width", 1)
      .style("cursor", "grab")
      .on("mouseenter", function (event, d) {
        el.tooltip.innerHTML = `${escapeHtml(d.kind)} · ${escapeHtml(d.type || "-")}<br><span class="mono">${escapeHtml(d.value || d.id)}</span>`;
        el.tooltip.style.opacity = "1";
        el.tooltip.style.left = `${event.offsetX}px`;
        el.tooltip.style.top = `${event.offsetY}px`;
      })
      .on("mousemove", function (event) {
        el.tooltip.style.left = `${event.offsetX}px`;
        el.tooltip.style.top = `${event.offsetY}px`;
      })
      .on("mouseleave", function () {
        el.tooltip.style.opacity = "0";
      })
      .on("click", function (event, d) {
        event.stopPropagation();
        if (d.cluster_id) {
          const c = (summary.top_clusters || []).find((x) => x.cluster_id === d.cluster_id);
          if (c) {
            el.inspector.innerHTML = makeInspectorHtml(c);
          } else {
            el.inspector.innerHTML = makeGraphFallbackInspector(d, graph);
          }
        } else {
          el.inspector.innerHTML = makeGraphFallbackInspector(d, graph);
        }
      })
      .call(
        d3
          .drag()
          .on("start", (event, d) => {
            if (!event.active) sim.alphaTarget(0.3).restart();
            d.fx = d.x;
            d.fy = d.y;
          })
          .on("drag", (event, d) => {
            d.fx = event.x;
            d.fy = event.y;
          })
          .on("end", (event, d) => {
            if (!event.active) sim.alphaTarget(0);
            d.fx = null;
            d.fy = null;
          })
      );

    sim.on("tick", () => {
      linkVis
        .attr("x1", (d) => d.source.x)
        .attr("y1", (d) => d.source.y)
        .attr("x2", (d) => d.target.x)
        .attr("y2", (d) => d.target.y);
      nodeVis.attr("cx", (d) => d.x).attr("cy", (d) => d.y);
    });
  }

  async function main() {
    try {
      const [summary, graph, report] = await Promise.all([
        loadJson(PATHS.summary),
        loadJson(PATHS.graph),
        loadText(PATHS.report),
      ]);

      renderSummary(summary);
      renderCaveats(report);

      const summaryRun = summary.run_id || "";
      const graphRun = graph && graph.run ? graph.run.run_id : "";
      const runOk = summaryRun === CANONICAL_RUN_ID && graphRun === CANONICAL_RUN_ID;
      if (!runOk) {
        setConsistency(
          false,
          "consistency: mismatch",
          `Expected canonical run ${CANONICAL_RUN_ID}. Got summary=${summaryRun || "missing"}, graph=${graphRun || "missing"}.`
        );
      } else {
        setConsistency(true, "consistency: canonical run aligned", "");
      }

      renderTable(summary, (clusterId) => {
        const c = (summary.top_clusters || []).find((x) => x.cluster_id === clusterId);
        if (c) {
          el.inspector.innerHTML = makeInspectorHtml(c);
          return;
        }
        const graphNode = (graph.nodes || []).find((n) => n.cluster_id === clusterId);
        if (graphNode) {
          el.inspector.innerHTML = makeGraphFallbackInspector(graphNode, graph);
          return;
        }
        el.inspector.innerHTML = `<p class="muted small">Cluster not found in summary or visible graph context.</p>`;
      });
      renderGraph(graph, summary);
    } catch (err) {
      setConsistency(false, "consistency: load failed", err.message || String(err));
    }
  }

  main();
})();
