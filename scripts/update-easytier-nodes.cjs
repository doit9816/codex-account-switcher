const fs = require("node:fs");
const https = require("node:https");

const source = process.env.EASYTIER_NODE_PAGE || "https://info.qtet.cn/uptime/easytier";
const output = process.env.EASYTIER_NODE_OUTPUT || "easytier-nodes.json";
const defaultPort = 11010;
const supportedSchemes = new Set(["tcp", "udp", "ws", "wss", "tcp+tls", "quic", "kcp"]);

class JsLiteralParser {
  constructor(input) {
    this.input = input;
    this.position = 0;
  }

  value() {
    this.skipWhitespace();
    const current = this.input[this.position];
    if (current === "{") return this.object();
    if (current === "[") return this.array();
    if (current === "'" || current === '"') return this.string();
    return this.bare();
  }

  object() {
    this.expect("{");
    const result = {};
    while (true) {
      this.skipWhitespace();
      if (this.consume("}")) return result;
      const key = this.input[this.position] === "'" || this.input[this.position] === '"'
        ? this.string()
        : this.identifier();
      this.skipWhitespace();
      this.expect(":");
      result[key] = this.value();
      this.skipWhitespace();
      if (this.consume("}")) return result;
      this.expect(",");
    }
  }

  array() {
    this.expect("[");
    const result = [];
    while (true) {
      this.skipWhitespace();
      if (this.consume("]")) return result;
      result.push(this.value());
      this.skipWhitespace();
      if (this.consume("]")) return result;
      this.expect(",");
    }
  }

  string() {
    const quote = this.input[this.position++];
    let result = "";
    while (this.position < this.input.length) {
      const current = this.input[this.position++];
      if (current === quote) return result;
      if (current !== "\\") {
        result += current;
        continue;
      }
      const escaped = this.input[this.position++];
      const escapes = { n: "\n", r: "\r", t: "\t", b: "\b", f: "\f", "\\": "\\", "/": "/", "'": "'", '"': '"' };
      if (escaped === "u") {
        result += String.fromCharCode(parseInt(this.input.slice(this.position, this.position + 4), 16));
        this.position += 4;
      } else {
        result += escapes[escaped] ?? escaped;
      }
    }
    throw new Error("unterminated JavaScript string");
  }

  identifier() {
    const start = this.position;
    while (/[A-Za-z0-9_$-]/.test(this.input[this.position] || "")) this.position += 1;
    if (this.position === start) throw new Error(`expected identifier at ${this.position}`);
    return this.input.slice(start, this.position);
  }

  bare() {
    const start = this.position;
    while (this.position < this.input.length && !/[\s,\]}]/.test(this.input[this.position])) this.position += 1;
    const token = this.input.slice(start, this.position);
    if (token === "true") return true;
    if (token === "false") return false;
    if (token === "null") return null;
    if (/^-?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(token)) return Number(token);
    return token;
  }

  skipWhitespace() {
    while (/\s/.test(this.input[this.position] || "")) this.position += 1;
  }

  expect(value) {
    if (!this.consume(value)) throw new Error(`expected ${value} at ${this.position}`);
  }

  consume(value) {
    if (this.input[this.position] === value) {
      this.position += 1;
      return true;
    }
    return false;
  }
}

function extractPublicGroupList(html) {
  const marker = "publicGroupList";
  const markerPosition = html.indexOf(marker);
  if (markerPosition < 0) throw new Error("publicGroupList not found");
  const arrayPosition = html.indexOf("[", markerPosition);
  if (arrayPosition < 0) throw new Error("publicGroupList array not found");
  return new JsLiteralParser(html.slice(arrayPosition)).value();
}

function normalizePeer(value) {
  const raw = String(value || "").trim();
  if (!raw || raw.includes("*")) return null;
  try {
    const candidate = raw.includes("://") ? raw : `tcp://${raw.includes(":") ? raw : `${raw}:${defaultPort}`}`;
    const parsed = new URL(candidate);
    if (!supportedSchemes.has(parsed.protocol.slice(0, -1)) || !parsed.hostname) return null;
    if (!parsed.port) parsed.port = String(defaultPort);
    return parsed.toString();
  } catch {
    return null;
  }
}

function nodeAddress(rawName, monitor) {
  const tags = [];
  const tagged = String(rawName || "").replace(/\[([^\]]+)\]/g, (_, tag) => {
    tags.push(tag.trim());
    return "";
  });
  const cleaned = tagged.split(/[（(]/, 1)[0].trim();
  const address = normalizePeer(cleaned);
  if (address) return { address, tags };
  if (monitor?.sendUrl === 1 && monitor?.url) {
    try {
      const parsed = new URL(monitor.url);
      return { address: `tcp://${parsed.hostname}:${parsed.port || defaultPort}`, tags };
    } catch {
      return { address: cleaned, tags };
    }
  }
  return { address: cleaned, tags };
}

async function fetchJson(url) {
  return JSON.parse(await fetchText(url));
}

async function fetchText(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { "user-agent": "codex-account-switcher-node-publisher" } }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => { body += chunk; });
      response.on("end", () => {
        if (response.statusCode < 200 || response.statusCode >= 300) {
          reject(new Error(`${url}: ${response.statusCode}`));
          return;
        }
        resolve(body);
      });
    }).on("error", reject);
  });
}

async function main() {
  const origin = source.replace(/\/uptime\/easytier\/?$/, "").replace(/\/$/, "");
  const statusHtml = await fetchText(`${origin}/uptime/status/easytier`);
  const heartbeat = await fetchJson(`${origin}/uptime/api/status-page/heartbeat/easytier`);
  const groups = extractPublicGroupList(statusHtml);
  const heartbeatList = heartbeat.heartbeatList || {};
  const uptimeList = heartbeat.uptimeList || {};
  const nodes = [];

  for (const group of groups) {
    for (const monitor of group.monitorList || []) {
      const parsed = nodeAddress(monitor.name, monitor);
      if (!parsed.address || parsed.address.includes("*")) continue;
      const id = String(monitor.id);
      const history = heartbeatList[id] || [];
      const latest = history[history.length - 1] || {};
      nodes.push({
        id,
        name: parsed.address,
        address: parsed.address,
        group: group.name || "EasyTier",
        status: latest.status === 1 ? "up" : "down",
        uptime24: typeof uptimeList[`${id}_24`] === "number" ? uptimeList[`${id}_24`] * 100 : null,
        pingMs: typeof latest.ping === "number" ? latest.ping : null,
        tags: parsed.tags,
      });
    }
  }

  nodes.sort((left, right) => Number(right.status === "up") - Number(left.status === "up") || left.name.localeCompare(right.name));
  const payload = {
    format: "codex-switcher.easytier-nodes.v1",
    source,
    updatedAt: new Date().toISOString(),
    nodes,
  };
  fs.writeFileSync(output, `${JSON.stringify(payload, null, 2)}\n`);
  console.log(`wrote ${nodes.length} nodes to ${output}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
