/* firespread detection mesh: polls the `serve` example's /api/alerts, draws
 * each node's bearing line, and triangulates a smoke fix when two or more
 * nodes agree. The mesh file is nodes/<mesh>.json, `?mesh=` picks it. */
(function () {
  "use strict";

  var POLL_MS = 1000;
  var RAY_M = 25000;
  // A ray whose perpendicular distance to the fix exceeds this is dropped as
  // an outlier, as long as two rays remain.
  var OUTLIER_M = 1500;
  // A fix whose rays miss it by more than this (RMS) is not shown.
  var MAX_RMS_M = 1000;
  // Rays closer to parallel than this give no usable crossing.
  var MIN_CROSS_DEG = 10;
  var COLORS = ["#1f77b4", "#2ca02c", "#9467bd", "#e377c2", "#17becf", "#8c564b", "#bcbd22"];

  var params = new URLSearchParams(location.search);
  var meshName = params.get("mesh") || "los-alamos";
  var connEl = document.getElementById("conn");
  var windowInput = document.getElementById("window-s");
  var confInput = document.getElementById("min-conf");

  var map = L.map("map", { zoomControl: true });
  // Online OSM tiles; without network the map keeps its plain background and
  // the nodes, rays and fix still draw.
  L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
    maxZoom: 17,
    attribution: "&copy; OpenStreetMap contributors",
  }).addTo(map);
  var rayLayer = L.layerGroup().addTo(map);
  var fixLayer = L.layerGroup().addTo(map);

  var nodes = {};
  var alerts = [];
  var lastSeq = 0;
  var resetSeq = 0;
  var firstAlertMs = null;
  var firstFixMs = null;

  function toRad(d) { return (d * Math.PI) / 180; }
  function toDeg(r) { return (r * 180) / Math.PI; }

  // Local east/north metres around (lat0, lon0); good to a few metres over
  // the 30 km of a mesh.
  function projector(lat0, lon0) {
    var kx = 111320 * Math.cos(toRad(lat0));
    var ky = 110540;
    return {
      fwd: function (lat, lon) { return [(lon - lon0) * kx, (lat - lat0) * ky]; },
      inv: function (x, y) { return [lat0 + y / ky, lon0 + x / kx]; },
    };
  }

  function destination(lat, lon, bearingDeg, distM) {
    var p = projector(lat, lon);
    var b = toRad(bearingDeg);
    return p.inv(Math.sin(b) * distM, Math.cos(b) * distM);
  }

  function fovWedge(n) {
    var pts = [[n.lat, n.lon]];
    for (var i = 0; i <= 12; i++) {
      var b = n.heading_deg - n.fov_deg / 2 + (n.fov_deg * i) / 12;
      pts.push(destination(n.lat, n.lon, b, 6000));
    }
    return pts;
  }

  function fmtTime(ms) {
    return new Date(ms).toLocaleTimeString([], { hour12: false });
  }

  // Weighted least-squares point closest to every ray, in local metres.
  // rays: [{p: [x, y], d: [dx, dy], w}]. Returns null when the rays are
  // close to parallel.
  function solve(rays) {
    var a11 = 0, a12 = 0, a22 = 0, b1 = 0, b2 = 0;
    rays.forEach(function (r) {
      var m11 = 1 - r.d[0] * r.d[0], m12 = -r.d[0] * r.d[1], m22 = 1 - r.d[1] * r.d[1];
      a11 += r.w * m11; a12 += r.w * m12; a22 += r.w * m22;
      b1 += r.w * (m11 * r.p[0] + m12 * r.p[1]);
      b2 += r.w * (m12 * r.p[0] + m22 * r.p[1]);
    });
    var det = a11 * a22 - a12 * a12;
    var wsum = rays.reduce(function (s, r) { return s + r.w; }, 0);
    if (det < 1e-6 * wsum * wsum) return null;
    return [(a22 * b1 - a12 * b2) / det, (a11 * b2 - a12 * b1) / det];
  }

  function perp(r, x) {
    var vx = x[0] - r.p[0], vy = x[1] - r.p[1];
    return { along: vx * r.d[0] + vy * r.d[1], off: Math.abs(vx * r.d[1] - vy * r.d[0]) };
  }

  function maxCrossDeg(rays) {
    var best = 0;
    for (var i = 0; i < rays.length; i++) {
      for (var j = i + 1; j < rays.length; j++) {
        var c = Math.abs(rays[i].d[0] * rays[j].d[1] - rays[i].d[1] * rays[j].d[0]);
        best = Math.max(best, toDeg(Math.asin(Math.min(1, c))));
      }
    }
    return best;
  }

  // Triangulates the latest active alert of each node. Drops rays that point
  // away from the fix, then the worst outlier, while two rays remain.
  function triangulate(active) {
    if (active.length < 2) return null;
    var lat0 = 0, lon0 = 0;
    active.forEach(function (a) { lat0 += a.lat; lon0 += a.lon; });
    var proj = projector(lat0 / active.length, lon0 / active.length);
    var rays = active.map(function (a) {
      var b = toRad(a.bearing_deg);
      return { a: a, p: proj.fwd(a.lat, a.lon), d: [Math.sin(b), Math.cos(b)], w: a.confidence };
    });
    while (rays.length >= 2) {
      if (maxCrossDeg(rays) < MIN_CROSS_DEG) return null;
      var x = solve(rays);
      if (!x) return null;
      var worst = -1, worstOff = -1, behind = -1, ss = 0;
      rays.forEach(function (r, i) {
        var q = perp(r, x);
        ss += q.off * q.off;
        if (q.along < 0) behind = i;
        if (q.off > worstOff) { worstOff = q.off; worst = i; }
      });
      if (behind >= 0) { rays.splice(behind, 1); continue; }
      if (worstOff > OUTLIER_M && rays.length > 2) { rays.splice(worst, 1); continue; }
      var rms = Math.sqrt(ss / rays.length);
      if (rms > MAX_RMS_M) return null;
      var ll = proj.inv(x[0], x[1]);
      return { lat: ll[0], lon: ll[1], rms: rms, used: rays.map(function (r) { return r.a; }) };
    }
    return null;
  }

  function activeAlerts(now) {
    var windowMs = Math.max(5, Number(windowInput.value) || 30) * 1000;
    var minConf = Number(confInput.value) || 0;
    var latest = {};
    alerts.forEach(function (a) {
      if (now - a.seenMs > windowMs || a.confidence < minConf) return;
      if (!latest[a.node_id] || latest[a.node_id].seq < a.seq) latest[a.node_id] = a;
    });
    return Object.keys(latest).map(function (k) { return latest[k]; });
  }

  function nodeColor(id) {
    return nodes[id] ? nodes[id].color : "#555";
  }

  function render() {
    var now = Date.now();
    var active = activeAlerts(now);
    var windowMs = Math.max(5, Number(windowInput.value) || 30) * 1000;
    // An episode starts with the first active alert after a quiet spell.
    if (active.length && firstAlertMs === null) {
      firstAlertMs = Math.min.apply(null, active.map(function (a) { return a.seenMs; }));
    }
    var fix = triangulate(active);
    var usedIds = fix ? fix.used.map(function (a) { return a.node_id; }) : [];

    rayLayer.clearLayers();
    active.forEach(function (a) {
      var age = (now - a.seenMs) / windowMs;
      var used = usedIds.indexOf(a.node_id) >= 0;
      L.polyline([[a.lat, a.lon], destination(a.lat, a.lon, a.bearing_deg, RAY_M)], {
        color: nodeColor(a.node_id),
        weight: used ? 4 : 3,
        opacity: Math.max(0.25, 1 - age * 0.75),
        dashArray: used ? null : "8 6",
      }).bindTooltip(a.node_id + " · " + a.label + " " + a.confidence.toFixed(2) + " · " + a.bearing_deg.toFixed(1) + "°")
        .addTo(rayLayer);
    });

    fixLayer.clearLayers();
    var card = document.getElementById("status-card");
    var label = document.getElementById("state-label");
    var detail = document.getElementById("state-detail");
    if (fix) {
      if (firstFixMs === null) firstFixMs = now;
      var radius = Math.max(250, 2 * fix.rms);
      L.circle([fix.lat, fix.lon], { radius: radius, color: "#b3261e", weight: 2, fillOpacity: 0.15 }).addTo(fixLayer);
      L.circleMarker([fix.lat, fix.lon], { radius: 7, color: "#fff", weight: 2, fillColor: "#b3261e", fillOpacity: 1 })
        .bindTooltip("smoke fix", { permanent: true, direction: "right", className: "fix-label" })
        .addTo(fixLayer);
      card.className = "state-fix";
      label.textContent = "Smoke confirmed by " + fix.used.length + " nodes";
      detail.innerHTML =
        fix.lat.toFixed(4) + ", " + fix.lon.toFixed(4) + " ± " + Math.round(radius) + " m<br>" +
        "nodes: " + usedIds.join(", ") + "<br>" +
        "first alert " + fmtTime(firstAlertMs) + ", confirmed " + fmtTime(firstFixMs) +
        " (" + Math.round((firstFixMs - firstAlertMs) / 1000) + " s)<br>" +
        '<a href="index.html" target="_blank" rel="noopener">burn-probability forecast</a> (served run, starts from its own perimeter, not this fix)';
    } else if (active.length > 0) {
      card.className = "state-single";
      label.textContent = "Unconfirmed: " + active.length + (active.length === 1 ? " node" : " nodes, no agreement");
      detail.textContent = active.map(function (a) {
        return a.node_id + " " + a.label + " " + a.confidence.toFixed(2) + " at " + a.bearing_deg.toFixed(0) + "°";
      }).join("; ") + ". Waiting for a second node to corroborate.";
    } else {
      card.className = "state-none";
      label.textContent = "No smoke reported";
      detail.textContent = alerts.length ? "Earlier alerts have aged out of the window." : "Nodes are watching.";
      // A new episode starts at the next alert.
      firstAlertMs = null;
      firstFixMs = null;
    }

    var tbody = document.querySelector("#node-table tbody");
    tbody.innerHTML = "";
    Object.keys(nodes).forEach(function (id) {
      var last = null;
      alerts.forEach(function (a) { if (a.node_id === id && (!last || a.seq > last.seq)) last = a; });
      var tr = document.createElement("tr");
      tr.innerHTML =
        '<td><span class="swatch" style="background:' + nodes[id].color + '"></span>' + id + "</td>" +
        "<td>" + (last ? Math.round((now - last.seenMs) / 1000) + " s ago" : "-") + "</td>" +
        "<td>" + (last ? last.confidence.toFixed(2) : "-") + "</td>" +
        "<td>" + (last ? last.bearing_deg.toFixed(0) + "°" : "-") + "</td>";
      tbody.appendChild(tr);
    });
  }

  function addFeed(a) {
    var li = document.createElement("li");
    li.textContent = fmtTime(a.seenMs) + " " + a.node_id + " " + a.label + " " +
      a.confidence.toFixed(2) + " " + a.bearing_deg.toFixed(0) + "° " + (a.source || "");
    var feed = document.getElementById("feed");
    feed.insertBefore(li, feed.firstChild);
    while (feed.childNodes.length > 60) feed.removeChild(feed.lastChild);
  }

  function clearLocal() {
    alerts = [];
    firstAlertMs = null;
    firstFixMs = null;
    document.getElementById("feed").innerHTML = "";
  }

  function poll() {
    fetch("api/alerts?after=" + lastSeq, { cache: "no-store" })
      .then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      })
      .then(function (page) {
        connEl.textContent = "live";
        connEl.className = "mono ok";
        if (page.reset_seq > resetSeq) {
          resetSeq = page.reset_seq;
          clearLocal();
        }
        var now = Date.now();
        page.alerts.forEach(function (a) {
          if (a.seq <= resetSeq) return;
          // Age alerts by the server's clock, so the clocks of the nodes and
          // of this browser need not agree.
          a.seenMs = now - Math.max(0, page.now_ms - a.received_ms);
          alerts.push(a);
          addFeed(a);
        });
        if (alerts.length > 2000) alerts = alerts.slice(-2000);
        lastSeq = Math.max(lastSeq, page.last_seq);
      })
      .catch(function (e) {
        connEl.textContent = "server unreachable (" + e.message + ")";
        connEl.className = "mono bad";
      })
      .then(function () {
        render();
        setTimeout(poll, POLL_MS);
      });
  }

  document.getElementById("reset").addEventListener("click", function () {
    fetch("api/alerts", { method: "DELETE" }).then(function () { clearLocal(); render(); });
  });

  fetch("nodes/" + encodeURIComponent(meshName) + ".json")
    .then(function (r) {
      if (!r.ok) throw new Error("mesh " + meshName + ": HTTP " + r.status);
      return r.json();
    })
    .then(function (mesh) {
      document.getElementById("mesh-name").textContent = mesh.name;
      document.getElementById("mesh-note").textContent = mesh.note || "";
      map.setView(mesh.center, mesh.zoom || 12);
      mesh.nodes.forEach(function (n, i) {
        n.color = COLORS[i % COLORS.length];
        nodes[n.id] = n;
        L.polygon(fovWedge(n), { color: n.color, weight: 1, fillOpacity: 0.07, interactive: false }).addTo(map);
        L.circleMarker([n.lat, n.lon], { radius: 8, color: "#fff", weight: 2, fillColor: n.color, fillOpacity: 1 })
          .bindTooltip(n.id + "<br>" + n.name, { permanent: true, direction: "top", offset: [0, -8] })
          .addTo(map);
      });
      poll();
    })
    .catch(function (e) {
      connEl.textContent = e.message;
      connEl.className = "mono bad";
    });
})();
