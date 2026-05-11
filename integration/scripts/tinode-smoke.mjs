#!/usr/bin/env node

const DEFAULT_ENDPOINT = "http://101.34.216.193:6060";
const DEFAULT_API_KEY = "AQEAAAABAAD_rAp4DJh05a1HAwFT3A6K";
const DEFAULT_PROTOCOL_VERSION = "0.25";
const DEFAULT_TIMEOUT_MS = 10_000;

function parseArgs(argv) {
  const args = {
    endpoint: process.env.TINODE_URL || DEFAULT_ENDPOINT,
    apiKey: process.env.TINODE_API_KEY || DEFAULT_API_KEY,
    prefix: process.env.TINODE_SMOKE_PREFIX || `appfs-smoke-${Date.now().toString(36)}`,
    password: process.env.TINODE_SMOKE_PASSWORD || "TinodeSmoke123!",
    protocolVersion: process.env.TINODE_PROTOCOL_VERSION || DEFAULT_PROTOCOL_VERSION,
    timeoutMs: Number(process.env.TINODE_TIMEOUT_MS || DEFAULT_TIMEOUT_MS),
    keep: false,
    verbose: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) {
        throw new Error(`missing value for ${arg}`);
      }
      i += 1;
      return argv[i];
    };

    if (arg === "--endpoint") {
      args.endpoint = next();
    } else if (arg === "--api-key") {
      args.apiKey = next();
    } else if (arg === "--prefix") {
      args.prefix = next();
    } else if (arg === "--password") {
      args.password = next();
    } else if (arg === "--protocol-version") {
      args.protocolVersion = next();
    } else if (arg === "--timeout-ms") {
      args.timeoutMs = Number(next());
    } else if (arg === "--keep") {
      args.keep = true;
    } else if (arg === "--verbose") {
      args.verbose = true;
    } else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }

  if (!Number.isFinite(args.timeoutMs) || args.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number");
  }
  return args;
}

function printHelp() {
  console.log(`Usage: node integration/scripts/tinode-smoke.mjs [options]

Options:
  --endpoint <url>     Tinode HTTP endpoint. Default: ${DEFAULT_ENDPOINT}
  --api-key <key>      Tinode API key. Default: Tinode demo key
  --prefix <prefix>    Unique test account prefix. Default: appfs-smoke-<timestamp>
  --password <secret>  Password for generated users. Default: TinodeSmoke123!
  --protocol-version <ver>
                       Tinode wire protocol version. Default: ${DEFAULT_PROTOCOL_VERSION}
  --timeout-ms <ms>    Per-step timeout. Default: ${DEFAULT_TIMEOUT_MS}
  --keep               Keep generated users and group for inspection
  --verbose            Print raw Tinode protocol messages
`);
}

function toWebSocketUrl(endpoint, apiKey) {
  const url = new URL(endpoint);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/v0/channels";
  url.searchParams.set("apikey", apiKey);
  return url.toString();
}

function base64(value) {
  return Buffer.from(value, "utf8").toString("base64");
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function makeTinodeLogin(prefix, suffix) {
  let base = prefix.toLowerCase().replace(/[^a-z0-9_.]/g, "");
  if (base.length === 0) {
    base = "appfssmoke";
  }
  if (!/^[a-z0-9]/.test(base)) {
    base = `u${base}`;
  }
  base = base.replace(/[^a-z0-9]$/g, "");
  if (base.length === 0) {
    base = "appfssmoke";
  }
  return `${base.slice(0, 28)}${suffix}`;
}

function stringify(value) {
  return JSON.stringify(value, null, 2);
}

class TinodeClient {
  constructor(name, options) {
    this.name = name;
    this.options = options;
    this.ws = null;
    this.backlog = [];
    this.waiters = [];
    this.userId = null;
    this.token = null;
  }

  async connect() {
    if (typeof WebSocket === "undefined") {
      throw new Error("This script requires Node.js with global WebSocket support, e.g. Node 22.");
    }

    const wsUrl = toWebSocketUrl(this.options.endpoint, this.options.apiKey);
    this.ws = new WebSocket(wsUrl);
    this.ws.addEventListener("message", (event) => {
      const text = String(event.data);
      if (this.options.verbose) {
        console.error(`[${this.name}] <= ${text}`);
      }
      let msg;
      try {
        msg = JSON.parse(text);
      } catch (error) {
        this.rejectWaiters(new Error(`failed to parse Tinode message: ${error.message}; text=${text}`));
        return;
      }
      this.routeMessage(msg);
    });
    this.ws.addEventListener("error", () => {
      this.rejectWaiters(new Error(`[${this.name}] websocket error`));
    });
    this.ws.addEventListener("close", () => {
      this.rejectWaiters(new Error(`[${this.name}] websocket closed`));
    });

    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`[${this.name}] websocket open timed out`)), this.options.timeoutMs);
      this.ws.addEventListener("open", () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
      this.ws.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error(`[${this.name}] websocket open failed`));
      }, { once: true });
    });

    await this.request("hi", {
      ver: this.options.protocolVersion,
      ua: "appfs-tinode-smoke/0.1",
    }, "hi");
  }

  close() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.close();
    }
  }

  rejectWaiters(error) {
    const waiters = this.waiters.splice(0);
    for (const waiter of waiters) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
  }

  routeMessage(msg) {
    for (let i = 0; i < this.waiters.length; i += 1) {
      const waiter = this.waiters[i];
      let matched = false;
      try {
        matched = waiter.predicate(msg);
      } catch (error) {
        this.waiters.splice(i, 1);
        clearTimeout(waiter.timer);
        waiter.reject(error);
        return;
      }
      if (matched) {
        this.waiters.splice(i, 1);
        clearTimeout(waiter.timer);
        waiter.resolve(msg);
        return;
      }
    }
    this.backlog.push(msg);
  }

  waitFor(predicate, label, timeoutMs = this.options.timeoutMs) {
    const existingIndex = this.backlog.findIndex(predicate);
    if (existingIndex >= 0) {
      const [msg] = this.backlog.splice(existingIndex, 1);
      return Promise.resolve(msg);
    }

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const index = this.waiters.findIndex((waiter) => waiter.timer === timer);
        if (index >= 0) {
          this.waiters.splice(index, 1);
        }
        reject(new Error(`[${this.name}] timed out waiting for ${label}`));
      }, timeoutMs);
      this.waiters.push({ predicate, resolve, reject, timer });
    });
  }

  send(packet) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error(`[${this.name}] websocket is not open`);
    }
    const text = JSON.stringify(packet);
    if (this.options.verbose) {
      console.error(`[${this.name}] => ${text}`);
    }
    this.ws.send(text);
  }

  async request(kind, payload, id) {
    const finalPayload = { ...payload, id };
    this.send({ [kind]: finalPayload });
    const ctrl = await this.waitForCtrl(id, kind);
    return ctrl.ctrl;
  }

  async waitForCtrl(id, label) {
    const msg = await this.waitFor((candidate) => {
      if (candidate.ctrl?.id === id) {
        return true;
      }
      return candidate.ctrl && candidate.ctrl.code >= 400 && !candidate.ctrl.id;
    }, `${label} ctrl`);
    const ctrl = msg.ctrl;
    if (ctrl.code >= 400) {
      throw new Error(`[${this.name}] ${label} failed: code=${ctrl.code} text=${ctrl.text || ""} params=${stringify(ctrl.params || {})}`);
    }
    return msg;
  }

  async requestMeta(kind, payload, id, topic) {
    const finalPayload = { ...payload, id };
    this.send({ [kind]: finalPayload });

    let meta = null;
    const deadline = Date.now() + this.options.timeoutMs;
    while (Date.now() < deadline) {
      const remaining = Math.max(1, deadline - Date.now());
      const msg = await this.waitFor((candidate) => {
        if (candidate.meta?.id === id) {
          return true;
        }
        if (candidate.meta && topic && candidate.meta.topic === topic) {
          return true;
        }
        if (candidate.ctrl?.id === id) {
          return true;
        }
        return false;
      }, `${kind} meta/ctrl`, remaining);

      if (msg.meta) {
        meta = msg.meta;
      }
      if (msg.ctrl?.id === id) {
        if (msg.ctrl.code >= 400) {
          throw new Error(`[${this.name}] ${kind} failed: code=${msg.ctrl.code} text=${msg.ctrl.text || ""}`);
        }
        return meta;
      }
      if (meta) {
        try {
          await this.waitForCtrl(id, kind);
        } catch {
          return meta;
        }
        return meta;
      }
    }

    if (meta) {
      return meta;
    }
    throw new Error(`[${this.name}] no meta returned for ${kind} ${id}`);
  }

  async createAccount({ login, password, displayName, tags }) {
    const ctrl = await this.request("acc", {
      user: "new",
      scheme: "basic",
      secret: base64(`${login}:${password}`),
      login: true,
      tags,
      desc: {
        public: { fn: displayName },
        defacs: {
          auth: "JRWPA",
          anon: "N",
        },
      },
    }, `acc-${login}`);

    this.userId = ctrl.params?.user;
    this.token = ctrl.params?.token;
    if (!this.userId) {
      throw new Error(`[${this.name}] account creation did not return params.user: ${stringify(ctrl)}`);
    }

    await this.request("sub", {
      topic: "me",
      get: { what: "desc sub" },
    }, `sub-me-${login}`);
  }

  async searchUser(query) {
    await this.request("sub", { topic: "fnd" }, `sub-fnd-${this.name}`);
    await this.request("set", {
      topic: "fnd",
      desc: { public: query },
    }, `set-fnd-${this.name}`);
    const meta = await this.requestMeta("get", {
      topic: "fnd",
      what: "sub",
    }, `get-fnd-${this.name}`, "fnd");
    const matches = Array.isArray(meta?.sub) ? meta.sub : [];
    await this.request("leave", { topic: "fnd" }, `leave-fnd-${this.name}`).catch(() => {});
    return matches;
  }

  async subscribeTopic(topic, options = {}) {
    const payload = { topic };
    if (options.set) {
      payload.set = options.set;
    }
    if (options.get) {
      payload.get = options.get;
    }
    const ctrl = await this.request("sub", payload, `sub-${topic}-${Date.now()}`);
    return ctrl.topic || topic;
  }

  async addUserToTopic(topic, userId) {
    await this.request("set", {
      topic,
      sub: {
        user: userId,
        mode: "JRWPA",
      },
    }, `invite-${topic}-${userId}`);
  }

  async publish(topic, content, id) {
    await this.request("pub", {
      topic,
      noecho: false,
      head: {
        mime: "text/plain",
      },
      content,
    }, id);
  }

  waitForData(topic, expectedText) {
    return this.waitFor((msg) => {
      const data = msg.data;
      if (!data || data.topic !== topic) {
        return false;
      }
      if (expectedText === undefined) {
        return true;
      }
      return data.content?.txt === expectedText || data.content?.text === expectedText;
    }, `data on ${topic}`, this.options.timeoutMs * 2);
  }

  async deleteTopic(topic) {
    await this.request("del", {
      topic,
      what: "topic",
      hard: true,
    }, `del-topic-${topic}`);
  }

  async deleteCurrentUser() {
    if (!this.userId) {
      return;
    }
    await this.request("del", {
      what: "user",
      hard: true,
    }, `del-user-${this.name}`);
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const userALogin = makeTinodeLogin(args.prefix, "a");
  const userBLogin = makeTinodeLogin(args.prefix, "b");
  const p2pText = `hello from AppFS smoke ${args.prefix}`;
  const groupText = `group hello from AppFS smoke ${args.prefix}`;

  const alice = new TinodeClient("alice", args);
  const bob = new TinodeClient("bob", args);
  let groupTopic = null;

  try {
    console.log(`Tinode smoke endpoint: ${args.endpoint}`);
    console.log(`Creating users: ${userALogin}, ${userBLogin}`);

    await alice.connect();
    await bob.connect();

    await alice.createAccount({
      login: userALogin,
      password: args.password,
      displayName: `AppFS Smoke A ${args.prefix}`,
      tags: [userALogin, args.prefix, "appfs-smoke"],
    });
    await bob.createAccount({
      login: userBLogin,
      password: args.password,
      displayName: `AppFS Smoke B ${args.prefix}`,
      tags: [userBLogin, args.prefix, "appfs-smoke"],
    });

    console.log(`Created users: ${alice.userId}, ${bob.userId}`);

    const matches = await alice.searchUser(`basic:${userBLogin}`);
    const bobMatch = matches.find((match) => match.topic === bob.userId || match.user === bob.userId);
    if (!bobMatch) {
      throw new Error(`search did not find ${userBLogin}; matches=${stringify(matches)}`);
    }
    console.log(`Search ok: basic:${userBLogin} -> ${bob.userId}`);

    await alice.subscribeTopic(bob.userId);
    await bob.subscribeTopic(alice.userId);
    const p2pData = bob.waitForData(alice.userId, p2pText);
    await alice.publish(bob.userId, { txt: p2pText }, "pub-p2p");
    const receivedP2p = await p2pData;
    console.log(`P2P send ok: ${receivedP2p.data.topic} seq=${receivedP2p.data.seq}`);

    groupTopic = await alice.subscribeTopic("new", {
      set: {
        desc: {
          public: { fn: `AppFS Smoke Group ${args.prefix}` },
          defacs: {
            auth: "JRWPA",
            anon: "N",
          },
        },
        tags: [args.prefix, "appfs-smoke-group"],
      },
    });
    console.log(`Group created: ${groupTopic}`);

    await alice.addUserToTopic(groupTopic, bob.userId);
    await bob.subscribeTopic(groupTopic);
    const groupData = bob.waitForData(groupTopic, groupText);
    await alice.publish(groupTopic, { txt: groupText }, "pub-group");
    const receivedGroup = await groupData;
    console.log(`Group send ok: ${receivedGroup.data.topic} seq=${receivedGroup.data.seq}`);

    console.log("Tinode smoke passed.");
  } finally {
    if (!args.keep) {
      console.log("Cleaning up generated Tinode objects...");
      if (groupTopic) {
        await alice.deleteTopic(groupTopic).catch((error) => console.warn(`cleanup group failed: ${error.message}`));
      }
      await delay(200);
      await bob.deleteCurrentUser().catch((error) => console.warn(`cleanup bob failed: ${error.message}`));
      await alice.deleteCurrentUser().catch((error) => console.warn(`cleanup alice failed: ${error.message}`));
    } else {
      console.log(`Keeping generated users and group. Password: ${args.password}`);
    }
    alice.close();
    bob.close();
  }
}

main().catch((error) => {
  console.error(`Tinode smoke failed: ${error.stack || error.message}`);
  process.exit(1);
});
