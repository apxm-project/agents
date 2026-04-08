// APXM GUI — Reactive Store + Event Bus
// Lightweight state management with path-based subscriptions and middleware.

(function () {
  'use strict';

  // -------------------------------------------------------------------------
  // Action type constants
  // -------------------------------------------------------------------------
  const Actions = {
    // UI
    SWITCH_TAB:     'ui/switchTab',
    SELECT_NODE:    'ui/selectNode',
    SET_LOADING:    'ui/setLoading',
    ADD_ERROR:      'ui/addError',
    CLEAR_ERRORS:   'ui/clearErrors',

    // Graph
    LOAD_GRAPH:     'graph/load',
    SET_ANALYSIS:   'graph/setAnalysis',
    SET_OPTIMIZED:  'graph/setOptimized',

    // Session
    LOAD_SESSION:   'session/load',

    // Live
    LIVE_START:     'live/start',
    LIVE_STOP:      'live/stop',
    LIVE_EVENT:     'live/event',
    LIVE_FOCUS_NODE:'live/focusNode',
    LIVE_NODE_START:'live/nodeStart',
    LIVE_NODE_END:  'live/nodeEnd',
    LIVE_TOKEN:     'live/token',
    LIVE_THOUGHT:   'live/thought',
    LIVE_TOOL_CALL: 'live/toolCall',
    LIVE_USAGE:     'live/usage',

    // Reference
    SET_OPS:        'ref/setOps',
    SET_CONFIG:     'ref/setConfig',
  };

  // -------------------------------------------------------------------------
  // Default initial state
  // -------------------------------------------------------------------------
  const DEFAULT_STATE = {
    ui: {
      activeTab: 'graph',
      selectedNode: null,
      sidebarOpen: true,
      loading: {},
      errors: [],
    },

    graph: {
      data: null,
      path: '',
      analysis: null,
      optimized: null,
      optimizationChanges: [],
    },

    session: {
      data: null,
      path: '',
      manifest: null,
      metrics: null,
      nodeStatuses: {},
      results: {},
    },

    live: {
      active: false,
      sessionPath: '',
      nodeStates: {},
      events: [],
      focusedNode: null,
      tokens: { input: 0, output: 0 },
      startTime: null,
      connection: null,
    },

    ops: [],
    config: null,
  };

  // -------------------------------------------------------------------------
  // Helpers
  // -------------------------------------------------------------------------

  // Keys that are non-serializable and excluded from getState snapshots.
  const TRANSIENT_KEYS = new Set(['live.connection']);

  function deepClone(obj) {
    if (obj === null || typeof obj !== 'object') return obj;
    try {
      return structuredClone(obj);
    } catch (_) {
      return JSON.parse(JSON.stringify(obj));
    }
  }

  /** Read a dot-path from an object. Returns undefined for missing segments. */
  function getPath(obj, path) {
    const keys = path.split('.');
    let cur = obj;
    for (const k of keys) {
      if (cur == null || typeof cur !== 'object') return undefined;
      cur = cur[k];
    }
    return cur;
  }

  /** Set a dot-path on an object, creating intermediate objects as needed. */
  function setPath(obj, path, value) {
    const keys = path.split('.');
    let cur = obj;
    for (let i = 0; i < keys.length - 1; i++) {
      const k = keys[i];
      if (cur[k] == null || typeof cur[k] !== 'object') {
        cur[k] = {};
      }
      cur = cur[k];
    }
    cur[keys[keys.length - 1]] = value;
  }

  /**
   * Check whether a subscription path matches a change path.
   *  - Exact match: 'ui.activeTab' matches 'ui.activeTab'
   *  - Prefix match: 'ui' matches 'ui.activeTab' (parent changed)
   *  - Child match:  'ui.activeTab' matches 'ui' (subscribing to child, parent was set)
   *  - Wildcard:     'session.nodes.*' matches 'session.nodes.3'
   */
  function pathMatches(subscriptionPath, changePath) {
    // Convert wildcard pattern to a check
    if (subscriptionPath.includes('*')) {
      const pattern = '^' + subscriptionPath.replace(/\./g, '\\.').replace(/\*/g, '[^.]+') + '(\\.|$)?';
      const re = new RegExp(pattern);
      if (re.test(changePath)) return true;
      // Also check if the change is a parent of the wildcard path
      const subParts = subscriptionPath.split('.');
      const changeParts = changePath.split('.');
      // changePath is a prefix of subscriptionPath (parent was set)
      if (changeParts.length < subParts.length) {
        let match = true;
        for (let i = 0; i < changeParts.length; i++) {
          if (subParts[i] !== '*' && subParts[i] !== changeParts[i]) {
            match = false;
            break;
          }
        }
        return match;
      }
      return false;
    }

    // Exact or prefix
    if (changePath === subscriptionPath) return true;
    if (changePath.startsWith(subscriptionPath + '.')) return true;
    if (subscriptionPath.startsWith(changePath + '.')) return true;
    return false;
  }

  /** Produce a frozen snapshot, omitting transient keys. */
  function snapshot(state) {
    const clone = deepClone(state);
    for (const path of TRANSIENT_KEYS) {
      setPath(clone, path, undefined);
    }
    return Object.freeze(clone);
  }

  // -------------------------------------------------------------------------
  // createStore
  // -------------------------------------------------------------------------
  function createStore(initialState) {
    const state = deepClone(initialState || DEFAULT_STATE);
    const subscribers = [];     // [{path, callback, id}]
    const middlewares = [];
    let subIdCounter = 0;

    function getState() {
      return snapshot(state);
    }

    function setState(path, value) {
      const prev = snapshot(state);
      setPath(state, path, value);
      notify(path, prev);
    }

    function notify(changedPath, prevSnapshot) {
      const cur = snapshot(state);
      for (const sub of subscribers) {
        if (pathMatches(sub.path, changedPath)) {
          try {
            sub.callback(getPath(cur, sub.path), getPath(prevSnapshot, sub.path), cur);
          } catch (err) {
            console.error('[Store] subscriber error on path "' + sub.path + '":', err);
          }
        }
      }
    }

    function subscribe(path, callback) {
      const id = ++subIdCounter;
      subscribers.push({ path, callback, id });
      return function unsubscribe() {
        const idx = subscribers.findIndex(function (s) { return s.id === id; });
        if (idx !== -1) subscribers.splice(idx, 1);
      };
    }

    function dispatch(action) {
      if (!action || typeof action.type !== 'string') {
        console.warn('[Store] dispatch called with invalid action:', action);
        return;
      }

      // Run middleware chain
      let idx = 0;
      function next(act) {
        if (idx < middlewares.length) {
          const mw = middlewares[idx++];
          mw(storeAPI, act, next);
        } else {
          applyAction(act);
        }
      }

      next(action);
    }

    function applyAction(action) {
      const prev = snapshot(state);
      const changed = reduce(state, action);

      // Notify subscribers for each changed path
      for (const path of changed) {
        for (const sub of subscribers) {
          if (pathMatches(sub.path, path)) {
            try {
              const cur = snapshot(state);
              sub.callback(getPath(cur, sub.path), getPath(prev, sub.path), cur);
            } catch (err) {
              console.error('[Store] subscriber error on path "' + sub.path + '":', err);
            }
          }
        }
      }
    }

    /** Built-in reducer. Returns array of changed paths. */
    function reduce(state, action) {
      const { type, payload } = action;

      switch (type) {
        // -- UI --
        case Actions.SWITCH_TAB:
          state.ui.activeTab = payload;
          return ['ui.activeTab'];

        case Actions.SELECT_NODE:
          state.ui.selectedNode = payload;
          return ['ui.selectedNode'];

        case Actions.SET_LOADING:
          state.ui.loading[payload.key] = payload.value;
          return ['ui.loading.' + payload.key, 'ui.loading'];

        case Actions.ADD_ERROR:
          state.ui.errors = state.ui.errors.concat(
            typeof payload === 'string' ? { message: payload, time: Date.now() } : payload
          );
          return ['ui.errors'];

        case Actions.CLEAR_ERRORS:
          state.ui.errors = [];
          return ['ui.errors'];

        // -- Graph --
        case Actions.LOAD_GRAPH:
          state.graph.data = payload.data || null;
          state.graph.path = payload.path || '';
          state.graph.analysis = null;
          state.graph.optimized = null;
          state.graph.optimizationChanges = [];
          return ['graph'];

        case Actions.SET_ANALYSIS:
          state.graph.analysis = payload;
          return ['graph.analysis'];

        case Actions.SET_OPTIMIZED:
          state.graph.optimized = payload.graph || null;
          state.graph.optimizationChanges = payload.changes || [];
          return ['graph.optimized', 'graph.optimizationChanges'];

        // -- Session --
        case Actions.LOAD_SESSION:
          state.session.data = payload.data || null;
          state.session.path = payload.path || '';
          state.session.manifest = payload.manifest || null;
          state.session.metrics = payload.metrics || null;
          state.session.nodeStatuses = payload.nodeStatuses || {};
          state.session.results = payload.results || {};
          return ['session'];

        // -- Live --
        case Actions.LIVE_START:
          state.live.active = true;
          state.live.sessionPath = payload.sessionPath || '';
          state.live.nodeStates = {};
          state.live.events = [];
          state.live.focusedNode = null;
          state.live.tokens = { input: 0, output: 0 };
          state.live.startTime = Date.now();
          state.live.connection = payload.connection || null;
          return ['live'];

        case Actions.LIVE_STOP:
          state.live.active = false;
          if (state.live.connection) {
            try { state.live.connection.close(); } catch (_) {}
          }
          state.live.connection = null;
          return ['live.active', 'live.connection'];

        case Actions.LIVE_EVENT:
          state.live.events = state.live.events.concat(payload);
          return ['live.events'];

        case Actions.LIVE_FOCUS_NODE:
          state.live.focusedNode = payload;
          return ['live.focusedNode'];

        case Actions.LIVE_NODE_START: {
          const id = payload.nodeId;
          const ns = state.live.nodeStates[id] || {};
          ns.status = 'running';
          ns.startTime = payload.time || Date.now();
          ns.output = ns.output || null;
          ns.thinking = ns.thinking || '';
          ns.tools = ns.tools || [];
          state.live.nodeStates[id] = ns;
          return ['live.nodeStates.' + id];
        }

        case Actions.LIVE_NODE_END: {
          const id = payload.nodeId;
          const ns = state.live.nodeStates[id] || {};
          ns.status = payload.status || 'completed';
          ns.endTime = payload.time || Date.now();
          ns.output = payload.output !== undefined ? payload.output : ns.output;
          state.live.nodeStates[id] = ns;
          return ['live.nodeStates.' + id];
        }

        case Actions.LIVE_TOKEN: {
          state.live.tokens.input += (payload.input || 0);
          state.live.tokens.output += (payload.output || 0);
          return ['live.tokens'];
        }

        case Actions.LIVE_THOUGHT: {
          const id = payload.nodeId;
          const ns = state.live.nodeStates[id] || { status: 'running', thinking: '', tools: [] };
          ns.thinking = (ns.thinking || '') + (payload.text || '');
          state.live.nodeStates[id] = ns;
          return ['live.nodeStates.' + id];
        }

        case Actions.LIVE_TOOL_CALL: {
          const id = payload.nodeId;
          const ns = state.live.nodeStates[id] || { status: 'running', thinking: '', tools: [] };
          ns.tools = (ns.tools || []).concat({
            name: payload.name,
            input: payload.input,
            output: payload.output,
            time: payload.time || Date.now(),
          });
          state.live.nodeStates[id] = ns;
          return ['live.nodeStates.' + id];
        }

        case Actions.LIVE_USAGE: {
          state.live.tokens.input += (payload.input || 0);
          state.live.tokens.output += (payload.output || 0);
          return ['live.tokens'];
        }

        // -- Reference --
        case Actions.SET_OPS:
          state.ops = payload;
          return ['ops'];

        case Actions.SET_CONFIG:
          state.config = payload;
          return ['config'];

        default:
          console.warn('[Store] unknown action type:', type);
          return [];
      }
    }

    function use(middleware) {
      middlewares.push(middleware);
    }

    const storeAPI = {
      getState,
      setState,
      subscribe,
      dispatch,
      use,
    };

    return storeAPI;
  }

  // -------------------------------------------------------------------------
  // createEventBus
  // -------------------------------------------------------------------------
  function createEventBus() {
    const handlers = {};  // event -> [{fn, id}]
    let idCounter = 0;

    function on(event, handler) {
      if (!handlers[event]) handlers[event] = [];
      const id = ++idCounter;
      handlers[event].push({ fn: handler, id: id });
      return function unsubscribe() {
        off(event, handler);
      };
    }

    function off(event, handler) {
      if (!handlers[event]) return;
      handlers[event] = handlers[event].filter(function (h) { return h.fn !== handler; });
    }

    function emit(event, data) {
      var list = handlers[event];
      if (!list) return;
      // Iterate over a copy in case handlers modify the list
      list.slice().forEach(function (h) {
        try {
          h.fn(data);
        } catch (err) {
          console.error('[EventBus] handler error for "' + event + '":', err);
        }
      });
    }

    function once(event, handler) {
      function wrapper(data) {
        off(event, wrapper);
        handler(data);
      }
      return on(event, wrapper);
    }

    return { on, off, emit, once };
  }

  // -------------------------------------------------------------------------
  // Built-in middleware: logging
  // -------------------------------------------------------------------------
  function loggingMiddleware(store, action, next) {
    if (!localStorage.getItem('apxm-debug')) {
      next(action);
      return;
    }
    var before = store.getState();
    console.group('[Store] ' + action.type);
    console.log('payload:', action.payload);
    console.log('before:', before);
    next(action);
    console.log('after:', store.getState());
    console.groupEnd();
  }

  // -------------------------------------------------------------------------
  // Built-in middleware: persistence
  // -------------------------------------------------------------------------
  var PERSIST_KEY = 'apxm-store-persist';

  function persistenceMiddleware(store, action, next) {
    next(action);
    var state = store.getState();
    try {
      var persisted = {
        'ui.activeTab': state.ui.activeTab,
        'graph.path': state.graph.path,
      };
      localStorage.setItem(PERSIST_KEY, JSON.stringify(persisted));
    } catch (_) {
      // localStorage may be unavailable
    }
  }

  /** Apply persisted values to an initial state object (mutates). */
  function restorePersisted(initialState) {
    try {
      var raw = localStorage.getItem(PERSIST_KEY);
      if (!raw) return;
      var persisted = JSON.parse(raw);
      if (persisted['ui.activeTab']) {
        setPath(initialState, 'ui.activeTab', persisted['ui.activeTab']);
      }
      if (persisted['graph.path']) {
        setPath(initialState, 'graph.path', persisted['graph.path']);
      }
    } catch (_) {
      // ignore
    }
  }

  // -------------------------------------------------------------------------
  // Export
  // -------------------------------------------------------------------------
  var defaultInitial = deepClone(DEFAULT_STATE);
  restorePersisted(defaultInitial);

  window.Store = {
    Actions:     Actions,
    createStore: createStore,
    createEventBus: createEventBus,

    // Pre-built middleware
    loggingMiddleware:     loggingMiddleware,
    persistenceMiddleware: persistenceMiddleware,

    // Convenience: a ready-to-use default store and bus
    store: (function () {
      var s = createStore(defaultInitial);
      s.use(loggingMiddleware);
      s.use(persistenceMiddleware);
      return s;
    })(),
    bus: createEventBus(),

    // Expose default state shape for reference / testing
    DEFAULT_STATE: Object.freeze(deepClone(DEFAULT_STATE)),
  };
})();
