/* firespread page: reads run/manifest.json and the run's JSON and GeoTIFF
 * files over http from the `serve` example, and draws the map and panels. */
(function () {
  "use strict";

  // EPSG:5070 (NAD83 / Conus Albers), from https://epsg.io/5070.proj4
  var EPSG5070 =
    "+proj=aea +lat_0=23 +lon_0=-96 +lat_1=29.5 +lat_2=45.5 +x_0=0 +y_0=0 " +
    "+ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs +type=crs";
  proj4.defs("EPSG:5070", EPSG5070);

  // ColorBrewer YlOrRd 5-class stops at p = 0, 0.25, 0.5, 0.75, 1.
  var RAMP = [
    [0.0, [255, 255, 178]],
    [0.25, [254, 204, 92]],
    [0.5, [253, 141, 60]],
    [0.75, [240, 59, 32]],
    [1.0, [189, 0, 38]],
  ];
  var OVERLAY_OPACITY = 0.75;
  var PMTILES_OPACITY = 0.45;
  var ENTRANTS = ["firespread", "persistence", "uniform_growth"];
  var SVG_NS = "http://www.w3.org/2000/svg";

  // Popup text per `data-help` key. `{brier_radius}` is the Brier domain
  // radius of the run's scorecard; `{horizon}` is its scored horizon.
  var HELP = {
    // Entrants
    firespread: "Our forecast. A cell counts as burned when at least half of the ensemble runs burn it by that hour.",
    persistence: "The fire does not move. The forecast is the observed perimeter at the window start.",
    uniform_growth: "The start perimeter pushed out evenly in every direction, at the growth rate of the 12 h before the window. It uses no wind, slope or fuel.",
    // Metrics
    jaccard: "The overlap of the forecast and observed burned areas, divided by their combined area. 1 is a perfect match and 0 is no overlap.",
    dice: "Twice the overlap, divided by the sum of the two areas. It ranks forecasts like Jaccard but gives higher numbers.",
    precision: "The share of the forecast burned area that really burned. A low value means the forecast burns too much.",
    recall: "The share of the observed burned area that the forecast caught. A low value means the forecast burns too little.",
    brier: "The mean squared error of the burn probability, over burnable cells within {brier_radius} of the start perimeter. 0 is perfect, and the baselines use only 0 or 1.",
    containment: "The share of each forecast's burned cells inside the official final fire perimeter (WFIGS). A low value means the forecast burns land that never burned.",
    // Map and charts
    ramp: "The color shows the share of ensemble runs that burn a cell by the selected hour. Cells that no run burns have no color.",
    slider: "Selects the hour of the map, counted from the window start. The scorecard scores only the hour of the observed perimeter.",
    areaChart: "The burned area of the ensemble runs by hour. The band holds the middle 80 % of runs, the line is the median, and the dots are the observed areas at the window start and end.",
    layerProbability: "Burn probability at the selected hour.",
    layerObserved: "The observed fire perimeter at the end of the window, from NASA FEDS satellite data.",
    layerConfusion: "Forecast against observed at h+{horizon}. Green is a hit: the forecast and FEDS both burned. Orange is a miss: FEDS burned and the forecast did not. Purple is a false alarm: the forecast burned and FEDS did not. The forecast is the cells with burn probability 0.5 or more.",
    layerHillshade: "Shaded terrain relief.",
    layerVector: "Roads, places and water. This layer shows only when the basemap file is present.",
  };

  var statusEl = document.getElementById("status");
  var brierRadius = "the scoring radius";
  var confusionHorizon = "N";

  function helpText(key) {
    var t = HELP[key];
    return t === undefined
      ? null
      : t.replace("{brier_radius}", brierRadius).replace("{horizon}", confusionHorizon);
  }

  // Shows the text of the nearest `[data-help]` ancestor on hover, keyboard
  // focus and tap; Escape or leaving the element hides it.
  function initHelp() {
    var tip = document.getElementById("help-tip");
    var owner = null;
    function hide() {
      if (owner) owner.removeAttribute("aria-describedby");
      owner = null;
      tip.hidden = true;
    }
    function show(target) {
      var s = helpText(target.getAttribute("data-help"));
      if (s === null) return;
      owner = target;
      tip.textContent = s;
      tip.hidden = false;
      target.setAttribute("aria-describedby", "help-tip");
      var r = target.getBoundingClientRect();
      var w = tip.offsetWidth;
      var h = tip.offsetHeight;
      var left = Math.min(Math.max(8, r.left), window.innerWidth - w - 8);
      var top = r.bottom + 6;
      if (top + h > window.innerHeight - 8) top = Math.max(8, r.top - h - 6);
      tip.style.left = left + "px";
      tip.style.top = top + "px";
    }
    function helpTarget(e) {
      return e.target && e.target.closest ? e.target.closest("[data-help]") : null;
    }
    document.addEventListener("mouseover", function (e) {
      var t = helpTarget(e);
      if (t && t !== owner) show(t);
    });
    document.addEventListener("mouseout", function (e) {
      var t = helpTarget(e);
      if (t && t === owner && !t.contains(e.relatedTarget)) hide();
    });
    document.addEventListener("focusin", function (e) {
      var t = helpTarget(e);
      if (t) show(t);
    });
    document.addEventListener("focusout", function (e) {
      if (helpTarget(e) === owner) hide();
    });
    document.addEventListener("click", function (e) {
      var t = helpTarget(e);
      if (t) show(t);
      else hide();
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape") hide();
    });
    window.addEventListener("scroll", hide, true);
  }

  // A layer-control label that carries popup text.
  function layerName(label, key) {
    return '<span tabindex="0" data-help="' + key + '">' + label + "</span>";
  }

  function setStatus(text) {
    statusEl.textContent = text;
  }

  function rampColor(p) {
    var x = Math.min(1, Math.max(0, p));
    for (var i = 1; i < RAMP.length; i++) {
      if (x <= RAMP[i][0]) {
        var a = RAMP[i - 1];
        var b = RAMP[i];
        var t = (x - a[0]) / (b[0] - a[0]);
        var c = [0, 1, 2].map(function (k) {
          return Math.round(a[1][k] + t * (b[1][k] - a[1][k]));
        });
        return "rgb(" + c.join(",") + ")";
      }
    }
    return "rgb(" + RAMP[RAMP.length - 1][1].join(",") + ")";
  }

  function drawLegend() {
    var stops = RAMP.map(function (s) {
      return "rgb(" + s[1].join(",") + ") " + s[0] * 100 + "%";
    });
    document.getElementById("legend-bar").style.background =
      "linear-gradient(to right, " + stops.join(", ") + ")";
  }

  // Fetches `url`; resolves null on a non-2xx status, a network failure or
  // unparseable JSON.
  function fetchOrNull(url, kind) {
    return fetch(url)
      .then(function (res) {
        if (!res.ok) return null;
        return kind === "json" ? res.json() : res.arrayBuffer();
      })
      .catch(function (e) {
        console.info("[firespread] " + url + ": " + e.message);
        return null;
      });
  }

  // Reads a single-band GeoTIFF with geotiff.js and wraps it as a georaster.
  async function loadGeoraster(url) {
    var buf = await fetchOrNull(url, "buffer");
    if (buf === null) throw new Error("missing " + url);
    var tiff = await GeoTIFF.fromArrayBuffer(buf);
    var image = await tiff.getImage();
    var bands = await image.readRasters({ interleave: false });
    var band = bands[0];
    var width = image.getWidth();
    var height = image.getHeight();
    var bbox = image.getBoundingBox();
    var res = image.getResolution();
    var keys = image.getGeoKeys() || {};
    var epsg = keys.ProjectedCSTypeGeoKey || keys.GeographicTypeGeoKey;
    var rows = [];
    for (var r = 0; r < height; r++) {
      rows.push(Array.prototype.slice.call(band.subarray(r * width, (r + 1) * width)));
    }
    return parseGeoraster([rows], {
      noDataValue: image.getGDALNoData(),
      projection: epsg,
      xmin: bbox[0],
      ymax: bbox[3],
      pixelWidth: Math.abs(res[0]),
      pixelHeight: Math.abs(res[1]),
    });
  }

  function probabilityLayer(georaster) {
    var nodata = georaster.noDataValue;
    return new GeoRasterLayer({
      georaster: georaster,
      opacity: OVERLAY_OPACITY,
      resolution: 128,
      pixelValuesToColorFn: function (values) {
        var p = values[0];
        if (p === nodata || !(p > 0)) return null;
        return rampColor(p);
      },
    });
  }

  function hillshadeLayer(georaster) {
    var nodata = georaster.noDataValue;
    return new GeoRasterLayer({
      georaster: georaster,
      opacity: 1,
      resolution: 256,
      pixelValuesToColorFn: function (values) {
        var v = values[0];
        if (v === nodata || v === null || isNaN(v)) return null;
        return "rgb(" + v + "," + v + "," + v + ")";
      },
    });
  }

  function gridBounds(grid) {
    var src = "EPSG:" + grid.epsg;
    var x0 = grid.origin_x;
    var y0 = grid.origin_y;
    var x1 = x0 + grid.nx * grid.dx;
    var y1 = y0 - grid.ny * grid.dy;
    var corners = [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ].map(function (c) {
      var ll = proj4(src, "EPSG:4326", c);
      return [ll[1], ll[0]];
    });
    return L.latLngBounds(corners);
  }

  async function addPmtiles(map, layers) {
    var url = "basemap/region.pmtiles";
    var head = await fetch(url, { method: "HEAD" }).then(
      function (r) {
        return r.ok;
      },
      function () {
        return false;
      }
    );
    if (!head) {
      console.info("[firespread] " + url + " not found; vector basemap skipped");
      return;
    }
    var archive = new pmtiles.PMTiles(url);
    await archive.getHeader();
    var layer = protomapsL.leafletLayer({ url: archive, flavor: "light", lang: "en" });
    layer.setOpacity(PMTILES_OPACITY);
    layer.addTo(map);
    layers.addOverlay(layer, layerName("Vector basemap", "layerVector"));
  }

  async function addHillshade(map, layers) {
    try {
      var g = await loadGeoraster("basemap/hillshade.tif");
      var layer = hillshadeLayer(g);
      layer.addTo(map);
      layer.bringToBack();
      layers.addOverlay(layer, layerName("Hillshade", "layerHillshade"));
    } catch (e) {
      console.info("[firespread] hillshade skipped: " + e.message);
    }
  }

  async function addObserved(map, layers) {
    var gj = await fetchOrNull("run/observed.geojson", "json");
    if (gj === null) {
      console.info("[firespread] run/observed.geojson not found");
      return;
    }
    var css = getComputedStyle(document.documentElement);
    var line = css.getPropertyValue("--observed-line").trim();
    var halo = css.getPropertyValue("--observed-halo").trim();
    var casing = L.geoJSON(gj, {
      style: { color: halo, weight: 5, opacity: 0.9, fill: false },
      interactive: false,
    });
    var stroke = L.geoJSON(gj, {
      style: { color: line, weight: 2.5, fill: false, dashArray: "6 4" },
    });
    var layer = L.layerGroup([casing, stroke]);
    layer.addTo(map);
    layers.addOverlay(layer, layerName("Observed perimeter", "layerObserved"));
  }

  // Confusion codes of `run/confusion_h<N>.tif`, N the scorecard's horizon:
  // 1 hit, 2 miss, 3 false alarm; 0 and nodata are transparent. Off by default.
  async function addConfusion(map, layers) {
    var card = await fetchOrNull("run/scorecard.json", "json");
    var row = card && Array.isArray(card.rows) ? card.rows.filter(isFeds)[0] : undefined;
    if (!row || row.horizon_h === undefined) {
      console.info("[firespread] no scored horizon; confusion map skipped");
      return;
    }
    var url = "run/confusion_h" + row.horizon_h + ".tif";
    var g;
    try {
      g = await loadGeoraster(url);
    } catch (e) {
      console.info("[firespread] confusion map skipped: " + e.message);
      return;
    }
    confusionHorizon = String(row.horizon_h);
    var css = getComputedStyle(document.documentElement);
    var colors = [
      null,
      css.getPropertyValue("--confusion-hit").trim(),
      css.getPropertyValue("--confusion-miss").trim(),
      css.getPropertyValue("--confusion-false-alarm").trim(),
    ];
    var nodata = g.noDataValue;
    var layer = new GeoRasterLayer({
      georaster: g,
      opacity: OVERLAY_OPACITY,
      resolution: 128,
      pixelValuesToColorFn: function (values) {
        var v = values[0];
        if (v === nodata) return null;
        return colors[v] || null;
      },
    });
    layers.addOverlay(layer, layerName("Confusion map", "layerConfusion"));
  }

  async function addProbability(map, layers, manifest) {
    var outputs = manifest.outputs || {};
    var hours = [];
    Object.keys(outputs).forEach(function (k) {
      var m = /^burn_probability_h(\d+)$/.exec(k);
      if (m) hours.push({ h: parseInt(m[1], 10), path: outputs[k] });
    });
    hours.sort(function (a, b) {
      return a.h - b.h;
    });
    var slider = document.getElementById("hour-slider");
    var label = document.getElementById("hour-label");
    if (hours.length === 0) {
      label.textContent = "no outputs";
      slider.disabled = true;
      return;
    }
    var rasters = await Promise.all(
      hours.map(function (e) {
        return loadGeoraster("run/" + e.path);
      })
    );
    var group = L.layerGroup().addTo(map);
    layers.addOverlay(group, layerName("Burn probability", "layerProbability"));
    // Every hour stays on the map so its tiles are drawn once; the slider
    // shows one hour by opacity.
    var built = rasters.map(probabilityLayer);
    built.forEach(function (layer) {
      layer.setOpacity(0);
      group.addLayer(layer);
    });
    function show(i) {
      built.forEach(function (layer, k) {
        layer.setOpacity(k === i ? OVERLAY_OPACITY : 0);
      });
      label.textContent = "h+" + hours[i].h;
    }
    slider.max = String(hours.length - 1);
    slider.value = "0";
    slider.addEventListener("input", function () {
      show(parseInt(slider.value, 10));
    });
    show(0);
  }

  // ---- inline SVG charts ----

  function el(name, attrs, parent) {
    var node = document.createElementNS(SVG_NS, name);
    Object.keys(attrs || {}).forEach(function (k) {
      node.setAttribute(k, attrs[k]);
    });
    if (parent) parent.appendChild(node);
    return node;
  }

  function text(parent, x, y, s, attrs) {
    var a = Object.assign({ x: x, y: y }, attrs || {});
    var t = el("text", a, parent);
    t.textContent = s;
    return t;
  }

  function notScored(container) {
    container.innerHTML = "";
    var p = document.createElement("div");
    p.className = "not-scored";
    p.textContent = "not yet scored";
    container.appendChild(p);
  }

  // Rows without an `observation` field (the fixture) count as FEDS rows.
  function isFeds(r) {
    return r.observation === undefined || r.observation === "feds";
  }

  // Share of each entrant's forecast cells inside the WFIGS final perimeter.
  function drawContainment(container, card) {
    var rows = card.rows.filter(function (r) {
      return r.observation === "wfigs_final";
    });
    if (rows.length === 0) return;
    var table = document.createElement("table");
    table.className = "containment";
    var cap = document.createElement("caption");
    cap.textContent = "Inside the final perimeter (WFIGS): share of forecast cells ";
    var help = document.createElement("button");
    help.type = "button";
    help.className = "help";
    help.setAttribute("data-help", "containment");
    help.setAttribute("aria-label", "About the final perimeter table");
    help.textContent = "?";
    cap.appendChild(help);
    table.appendChild(cap);
    rows.forEach(function (r) {
      var tr = document.createElement("tr");
      var th = document.createElement("th");
      th.textContent = r.entrant + " h+" + r.horizon_h;
      var td = document.createElement("td");
      td.textContent = r.precision === null || r.precision === undefined
        ? "n/a"
        : (100 * r.precision).toFixed(1) + " %";
      tr.appendChild(th);
      tr.appendChild(td);
      table.appendChild(tr);
    });
    container.appendChild(table);
  }

  function drawScorecard(container, card, metric) {
    container.innerHTML = "";
    var W = 340;
    var H = 210;
    var m = { l: 34, r: 6, t: 26, b: 28 };
    var svg = el("svg", { class: "chart", viewBox: "0 0 " + W + " " + H, role: "img" }, container);
    var horizons = [];
    card.rows.filter(isFeds).forEach(function (r) {
      if (horizons.indexOf(r.horizon_h) < 0) horizons.push(r.horizon_h);
    });
    horizons.sort(function (a, b) {
      return a - b;
    });
    var iw = W - m.l - m.r;
    var ih = H - m.t - m.b;
    var y = function (v) {
      return m.t + ih * (1 - v);
    };
    [0, 0.25, 0.5, 0.75, 1].forEach(function (v) {
      el("line", { class: "grid", x1: m.l, x2: W - m.r, y1: y(v), y2: y(v) }, svg);
      text(svg, m.l - 4, y(v) + 4, String(v), { class: "tick-label", "text-anchor": "end" });
    });
    var gw = iw / Math.max(1, horizons.length);
    var bw = (gw * 0.8) / ENTRANTS.length;
    horizons.forEach(function (h, gi) {
      var gx = m.l + gi * gw + gw * 0.1;
      ENTRANTS.forEach(function (e, ei) {
        var row = card.rows.filter(function (r) {
          return r.horizon_h === h && r.entrant === e && isFeds(r);
        })[0];
        var x = gx + ei * bw;
        var v = row ? row[metric] : null;
        if (v === null || v === undefined) {
          text(svg, x + bw / 2, y(0) - 3, "n/a", {
            class: "tick-label",
            "text-anchor": "middle",
            "font-size": "8",
          });
          return;
        }
        var vv = Math.min(1, Math.max(0, v));
        var bar = el(
          "rect",
          { class: "s-" + e, x: x, y: y(vv), width: Math.max(1, bw - 1), height: y(0) - y(vv) },
          svg
        );
        el("title", {}, bar).textContent = e + " h+" + h + " " + metric + " = " + v;
      });
      text(svg, m.l + gi * gw + gw / 2, H - m.b + 14, "h+" + h, {
        class: "tick-label",
        "text-anchor": "middle",
      });
    });
    el("line", { class: "axis", x1: m.l, x2: W - m.r, y1: y(0), y2: y(0) }, svg);
    var lx = m.l;
    ENTRANTS.forEach(function (e) {
      var g = el("g", { class: "help-target", tabindex: "0", "data-help": e }, svg);
      el("rect", { class: "s-" + e, x: lx, y: 6, width: 10, height: 10 }, g);
      text(g, lx + 14, 15, e);
      lx += 14 + e.length * 6.2 + 12;
    });
    text(svg, W - m.r, H - 2, "forecast horizon", { class: "axis-label", "text-anchor": "end" });
  }

  function niceMax(v) {
    if (!(v > 0)) return 1;
    var p = Math.pow(10, Math.floor(Math.log10(v)));
    var f = v / p;
    var n = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
    return n * p;
  }

  function hhmm(ms) {
    var d = new Date(ms);
    var pad = function (n) {
      return (n < 10 ? "0" : "") + n;
    };
    return pad(d.getUTCHours()) + ":" + pad(d.getUTCMinutes());
  }

  function drawArea(container, area) {
    container.innerHTML = "";
    var W = 340;
    var H = 210;
    var m = { l: 40, r: 8, t: 22, b: 30 };
    var svg = el("svg", { class: "chart", viewBox: "0 0 " + W + " " + H, role: "img" }, container);
    var ts = area.hours_utc.map(function (s) {
      return Date.parse(s);
    });
    var ens = area.ensemble_km2;
    var obs = (area.observed || []).map(function (o) {
      return { t: Date.parse(o.t), a: o.area_km2 };
    });
    var allT = ts.concat(
      obs.map(function (o) {
        return o.t;
      })
    );
    var t0 = Math.min.apply(null, allT);
    var t1 = Math.max.apply(null, allT);
    if (t1 === t0) t1 = t0 + 3600 * 1000;
    var ymax = niceMax(
      Math.max.apply(
        null,
        ens.p90.concat(
          obs.map(function (o) {
            return o.a;
          })
        )
      )
    );
    var iw = W - m.l - m.r;
    var ih = H - m.t - m.b;
    var x = function (t) {
      return m.l + (iw * (t - t0)) / (t1 - t0);
    };
    var y = function (v) {
      return m.t + ih * (1 - v / ymax);
    };
    [0, 0.25, 0.5, 0.75, 1].forEach(function (f) {
      var v = f * ymax;
      el("line", { class: "grid", x1: m.l, x2: W - m.r, y1: y(v), y2: y(v) }, svg);
      text(svg, m.l - 4, y(v) + 4, String(Math.round(v * 100) / 100), {
        class: "tick-label",
        "text-anchor": "end",
      });
    });
    var upper = ts.map(function (t, i) {
      return x(t) + "," + y(ens.p90[i]);
    });
    var lower = ts
      .map(function (t, i) {
        return x(t) + "," + y(ens.p10[i]);
      })
      .reverse();
    el("polygon", { class: "band", points: upper.concat(lower).join(" ") }, svg);
    el(
      "polyline",
      {
        class: "median",
        points: ts
          .map(function (t, i) {
            return x(t) + "," + y(ens.p50[i]);
          })
          .join(" "),
      },
      svg
    );
    obs.forEach(function (o) {
      var c = el("circle", { class: "obs", cx: x(o.t), cy: y(o.a), r: 3.5 }, svg);
      el("title", {}, c).textContent = "observed " + new Date(o.t).toISOString() + " " + o.a + " km2";
    });
    el("line", { class: "axis", x1: m.l, x2: W - m.r, y1: y(0), y2: y(0) }, svg);
    var nTicks = 4;
    for (var k = 0; k <= nTicks; k++) {
      var t = t0 + ((t1 - t0) * k) / nTicks;
      text(svg, x(t), H - m.b + 14, hhmm(t), { class: "tick-label", "text-anchor": "middle" });
    }
    text(svg, W - m.r, H - 2, "UTC", { class: "axis-label", "text-anchor": "end" });
    text(svg, m.l, 12, "km²: p10–p90 band, p50 line, observed points", {
      class: "axis-label",
    });
  }

  async function loadPanels() {
    var scEl = document.getElementById("scorecard-chart");
    var metricEl = document.getElementById("metric");
    var card = await fetchOrNull("run/scorecard.json", "json");
    if (card === null || !Array.isArray(card.rows)) {
      notScored(scEl);
      metricEl.disabled = true;
    } else {
      var radius = card.brier_domain && card.brier_domain.radius_m;
      if (radius > 0) brierRadius = radius / 1000 + " km";
      var redraw = function () {
        metricEl.setAttribute("data-help", metricEl.value);
        drawScorecard(scEl, card, metricEl.value);
        drawContainment(scEl, card);
      };
      metricEl.addEventListener("change", redraw);
      redraw();
    }
    var areaEl = document.getElementById("area-chart");
    var area = await fetchOrNull("run/area_by_hour.json", "json");
    if (area === null || !Array.isArray(area.hours_utc) || !area.ensemble_km2) {
      notScored(areaEl);
    } else {
      drawArea(areaEl, area);
    }
  }

  async function main() {
    initHelp();
    drawLegend();
    var panels = loadPanels();
    var manifest = await fetchOrNull("run/manifest.json", "json");
    if (manifest === null) {
      setStatus("run/manifest.json not found");
      await panels;
      return;
    }
    var runId = String(manifest.run_id || "");
    document.getElementById("run-id").textContent = runId;
    document.getElementById("window").textContent = manifest.window
      ? manifest.window.name + " " + manifest.window.start_t + " → " + manifest.window.target_t
      : "-";
    if (runId.indexOf("fixture") === 0) {
      document.getElementById("fixture-banner").classList.add("shown");
    }
    var map = L.map("map", { zoomControl: true });
    if (manifest.grid) {
      map.fitBounds(gridBounds(manifest.grid));
    } else {
      map.setView([35.85, -105.4], 10);
    }
    var layers = L.control.layers(null, null, { collapsed: false }).addTo(map);
    await addHillshade(map, layers);
    try {
      await addPmtiles(map, layers);
    } catch (e) {
      console.info("[firespread] vector basemap skipped: " + e.message);
    }
    await addProbability(map, layers, manifest);
    await addObserved(map, layers);
    await addConfusion(map, layers);
    await panels;
    setStatus("ready");
    document.body.setAttribute("data-ready", "1");
  }

  main().catch(function (e) {
    setStatus("error: " + e.message);
    console.error(e);
  });
})();
