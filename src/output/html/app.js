/* The diagram of the modfence HTML output. It draws the graph embedded in
   #modfence-data with the vendored d3-force (global `d3`), and needs no
   network. The static tables in the page are the same graph without it. */
(function () {
  'use strict';

  var NS = 'http://www.w3.org/2000/svg';
  var WIDTH = 960;
  var HEIGHT = 640;
  var data = JSON.parse(document.getElementById('modfence-data').textContent);
  var svg = document.getElementById('diagram');
  var viewport = document.getElementById('viewport');
  var status = document.getElementById('status');
  var motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  var SHAPES = {
    package: 'M-9,-9h18v18h-18z',
    module: 'M-8,0a8,8 0 1,0 16,0a8,8 0 1,0 -16,0',
    external: 'M0,-11L11,0L0,11L-11,0z',
    symbol: 'M0,-9L9,8H-9z'
  };

  var nodes = data.nodes.map(function (n) {
    return { id: n.id, info: n, cluster: n.cluster };
  });
  var byId = Object.create(null);
  nodes.forEach(function (n) { byId[n.id] = n; });
  var links = data.edges.map(function (e) {
    return { source: e.source, target: e.target, info: e };
  });

  function el(name, attrs, parent) {
    var node = document.createElementNS(NS, name);
    Object.keys(attrs || {}).forEach(function (key) { node.setAttribute(key, attrs[key]); });
    if (parent) { parent.appendChild(node); }
    return node;
  }

  function describe(info) {
    var lines = [info.label + ' (' + info.kind + ', ' + info.lang + ')'];
    if (info.path) { lines.push(info.path); }
    Object.keys(info.tags).sort().forEach(function (key) {
      lines.push(key + ': ' + info.tags[key].join(', '));
    });
    return lines.join('\n');
  }

  var clusterLayer = el('g', {}, viewport);
  var edgeLayer = el('g', {}, viewport);
  var nodeLayer = el('g', {}, viewport);

  var boxes = data.clusters.map(function (c) {
    var box = el('rect', { 'class': 'cluster', rx: 8 }, clusterLayer);
    var label = el('text', { 'class': 'cluster-label halo' }, clusterLayer);
    label.textContent = c.label;
    var members = c.members.map(function (id) { return byId[id]; }).filter(Boolean);
    return { members: members, box: box, label: label };
  });

  var lines = links.map(function (l) {
    var line = el('line', {
      'class': 'edge ' + l.info.relation,
      'marker-end': 'url(#arrow)'
    }, edgeLayer);
    var title = el('title', {}, line);
    title.textContent = l.info.source + ' ' + l.info.relation + ' ' + l.info.target +
      (l.info.count ? ' (' + l.info.count + ' sites' +
        (l.info.confidence ? ', ' + l.info.confidence : '') + ')' : '');
    return line;
  });

  nodes.forEach(function (n) {
    n.group = el('g', { 'class': 'node' }, nodeLayer);
    el('path', { 'class': 'shape k-' + n.info.kind, d: SHAPES[n.info.kind] || SHAPES.module }, n.group);
    var text = el('text', { x: 14, y: 4, 'class': 'halo' }, n.group);
    text.textContent = n.info.label;
    el('title', {}, n.group).textContent = describe(n.info);
  });

  /* Selection and filter. */
  var selected = null;
  var buttons = Array.prototype.slice.call(document.querySelectorAll('.node-btn'));

  function neighbours(id) {
    var out = Object.create(null);
    links.forEach(function (l) {
      if (l.info.source === id) { out[l.info.target] = 'to'; }
      if (l.info.target === id) { out[l.info.source] = out[l.info.source] || 'from'; }
    });
    return out;
  }

  /* The live region holds the filter note, then the selection description. */
  var note = '';
  function say(text) {
    status.textContent = [note, text].filter(Boolean).join(' ');
  }

  function names(ids) {
    return ids.map(function (id) { return byId[id].info.label; }).join(', ');
  }

  function paint() {
    var near = selected ? neighbours(selected) : {};
    nodes.forEach(function (n) {
      n.group.setAttribute('class', 'node' +
        (n.id === selected ? ' sel' : '') + (near[n.id] ? ' nbr' : '') + (n.off ? ' off' : ''));
    });
    links.forEach(function (l, i) {
      var hot = selected !== null && (l.info.source === selected || l.info.target === selected);
      lines[i].setAttribute('class', 'edge ' + l.info.relation +
        (hot ? ' hot' : '') + (selected !== null && !hot ? ' faded' : ''));
      lines[i].setAttribute('marker-end', hot ? 'url(#arrow-hot)' : 'url(#arrow)');
    });
    buttons.forEach(function (b) {
      var on = b.getAttribute('data-id') === selected;
      b.setAttribute('aria-pressed', on ? 'true' : 'false');
      b.closest('tr').setAttribute('class', on ? 'picked' : '');
    });
    if (selected === null) {
      say('');
      return;
    }
    var out = [];
    var into = [];
    links.forEach(function (l) {
      if (l.info.source === selected) { out.push(l.info.target); }
      if (l.info.target === selected) { into.push(l.info.source); }
    });
    say(byId[selected].info.label + ' selected. Points to: ' +
      (out.length ? names(out) : 'nothing') + '. Pointed to by: ' +
      (into.length ? names(into) : 'nothing') + '.');
  }

  function select(id) {
    selected = id === selected ? null : id;
    paint();
  }

  buttons.forEach(function (b) {
    b.addEventListener('click', function () { select(b.getAttribute('data-id')); });
  });

  var keySelect = document.getElementById('tag-key');
  var valueSelect = document.getElementById('tag-value');
  var tags = Object.create(null);
  nodes.forEach(function (n) {
    Object.keys(n.info.tags).forEach(function (key) {
      tags[key] = tags[key] || Object.create(null);
      n.info.tags[key].forEach(function (value) { tags[key][value] = true; });
    });
  });
  Object.keys(tags).sort().forEach(function (key) {
    var option = document.createElement('option');
    option.value = key;
    option.textContent = key;
    keySelect.appendChild(option);
  });

  function fillValues() {
    valueSelect.textContent = '';
    var any = document.createElement('option');
    any.value = '';
    any.textContent = 'any value';
    valueSelect.appendChild(any);
    Object.keys(tags[keySelect.value] || {}).sort().forEach(function (value) {
      var option = document.createElement('option');
      option.value = value;
      option.textContent = value;
      valueSelect.appendChild(option);
    });
    valueSelect.disabled = keySelect.value === '';
  }

  function filter() {
    var key = keySelect.value;
    var value = valueSelect.value;
    var shown = 0;
    nodes.forEach(function (n) {
      var values = key ? n.info.tags[key] : null;
      n.off = key !== '' && !(values && (value === '' || values.indexOf(value) >= 0));
      if (!n.off) { shown += 1; }
    });
    buttons.forEach(function (b) {
      b.closest('tr').hidden = byId[b.getAttribute('data-id')].off;
    });
    note = key ? shown + ' of ' + nodes.length + ' nodes match.' : '';
    paint();
  }

  keySelect.addEventListener('change', function () { fillValues(); filter(); });
  valueSelect.addEventListener('change', filter);
  fillValues();

  /* Layout. */
  function clusterForce() {
    function force(alpha) {
      boxes.forEach(function (c) {
        if (!c.members.length) { return; }
        var cx = 0;
        var cy = 0;
        c.members.forEach(function (n) { cx += n.x; cy += n.y; });
        cx /= c.members.length;
        cy /= c.members.length;
        c.members.forEach(function (n) {
          n.vx += (cx - n.x) * 0.15 * alpha;
          n.vy += (cy - n.y) * 0.15 * alpha;
        });
      });
    }
    return force;
  }

  var simulation = d3.forceSimulation(nodes)
    .force('link', d3.forceLink(links).id(function (n) { return n.id; })
      .distance(function (l) { return l.info.relation === 'contains' ? 50 : 90; }))
    .force('charge', d3.forceManyBody().strength(-260))
    .force('centre', d3.forceCenter(WIDTH / 2, HEIGHT / 2))
    .force('collide', d3.forceCollide(22))
    .force('cluster', clusterForce());

  function draw() {
    links.forEach(function (l, i) {
      var dx = l.target.x - l.source.x;
      var dy = l.target.y - l.source.y;
      var len = Math.sqrt(dx * dx + dy * dy) || 1;
      var line = lines[i];
      line.setAttribute('x1', l.source.x);
      line.setAttribute('y1', l.source.y);
      line.setAttribute('x2', l.target.x - (dx / len) * 14);
      line.setAttribute('y2', l.target.y - (dy / len) * 14);
    });
    nodes.forEach(function (n) {
      n.group.setAttribute('transform', 'translate(' + n.x + ',' + n.y + ')');
    });
    boxes.forEach(function (c) {
      if (!c.members.length) { return; }
      var xs = c.members.map(function (n) { return n.x; });
      var ys = c.members.map(function (n) { return n.y; });
      var x0 = Math.min.apply(null, xs) - 30;
      var y0 = Math.min.apply(null, ys) - 30;
      c.box.setAttribute('x', x0);
      c.box.setAttribute('y', y0);
      c.box.setAttribute('width', Math.max.apply(null, xs) + 30 - x0);
      c.box.setAttribute('height', Math.max.apply(null, ys) + 30 - y0);
      c.label.setAttribute('x', x0 + 8);
      c.label.setAttribute('y', y0 + 14);
    });
  }

  function settle() {
    var ticks = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
    simulation.stop();
    for (var i = 0; i < ticks; i += 1) { simulation.tick(); }
    draw();
  }

  /* Pan, zoom and drag. */
  var view = { x: 0, y: 0, k: 1 };

  function apply() {
    viewport.setAttribute('transform', 'translate(' + view.x + ',' + view.y + ') scale(' + view.k + ')');
  }

  function toSvg(event) {
    var point = svg.createSVGPoint();
    point.x = event.clientX;
    point.y = event.clientY;
    return point.matrixTransform(svg.getScreenCTM().inverse());
  }

  function zoom(factor, at) {
    var k = Math.min(8, Math.max(0.2, view.k * factor));
    var centre = at || { x: WIDTH / 2, y: HEIGHT / 2 };
    view.x = centre.x - ((centre.x - view.x) / view.k) * k;
    view.y = centre.y - ((centre.y - view.y) / view.k) * k;
    view.k = k;
    apply();
  }

  document.getElementById('zoom-in').addEventListener('click', function () { zoom(1.25); });
  document.getElementById('zoom-out').addEventListener('click', function () { zoom(0.8); });
  document.getElementById('zoom-reset').addEventListener('click', function () {
    view = { x: 0, y: 0, k: 1 };
    apply();
  });
  svg.addEventListener('wheel', function (event) {
    event.preventDefault();
    zoom(event.deltaY < 0 ? 1.1 : 1 / 1.1, toSvg(event));
  }, { passive: false });

  var drag = null;

  svg.addEventListener('pointerdown', function (event) {
    if (!event.isPrimary || event.button !== 0 || drag) { return; }
    var group = event.target.closest('.node');
    var start = toSvg(event);
    drag = { start: start, moved: false, view: { x: view.x, y: view.y }, node: null };
    if (group) {
      drag.node = nodes.filter(function (n) { return n.group === group; })[0];
    }
    svg.setPointerCapture(event.pointerId);
  });

  svg.addEventListener('pointermove', function (event) {
    if (!drag) { return; }
    var now = toSvg(event);
    var dx = now.x - drag.start.x;
    var dy = now.y - drag.start.y;
    if (!drag.moved && Math.abs(dx) + Math.abs(dy) < 4) { return; }
    drag.moved = true;
    if (drag.node) {
      drag.node.fx = (now.x - view.x) / view.k;
      drag.node.fy = (now.y - view.y) / view.k;
      if (motion.matches) {
        drag.node.x = drag.node.fx;
        drag.node.y = drag.node.fy;
        draw();
      } else {
        simulation.alphaTarget(0.3).restart();
      }
    } else {
      view.x = drag.view.x + dx;
      view.y = drag.view.y + dy;
      apply();
    }
  });

  function release() {
    if (!drag) { return; }
    var done = drag;
    drag = null;
    if (done.node) {
      done.node.fx = null;
      done.node.fy = null;
      simulation.alphaTarget(0);
    }
    if (!done.moved) { select(done.node ? done.node.id : null); }
  }

  svg.addEventListener('pointerup', release);
  svg.addEventListener('pointercancel', release);

  simulation.on('tick', draw);
  if (motion.matches) {
    settle();
  } else {
    simulation.alpha(1).restart();
  }
  motion.addEventListener('change', function () {
    if (motion.matches) { settle(); }
  });
  apply();
}());
