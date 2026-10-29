const assert = require('node:assert/strict');
const http = require('node:http');
const { createRequire } = require('node:module');
const { resolve } = require('node:path');
const { once } = require('node:events');
const { spawnSync } = require('node:child_process');

async function main() {
  const scope = spawnSync(process.execPath, ['-e', `
    const assert = require('node:assert/strict');
    const { createRequire } = require('node:module');
    const { resolve } = require('node:path');
    const undici = createRequire(resolve(process.env.WEBCODEX_MCP_CONFORMANCE_HARNESS_BUILD, 'package.json'))('undici');
    undici.request = (url, opts) => opts.headers?.authorization;
    globalThis.fetch = request => request.headers.get('authorization');
    require('./scripts/mcp_conformance_auth.cjs');
    for (const [url, allowed] of [
      ['http://127.0.0.1:1234/mcp', true], ['http://localhost:1234/mcp', true],
      ['http://[::1]:1234/mcp', true], ['http://evil.example:1234/mcp', false],
      ['http://localhost.evil.example:1234/mcp', false], ['http://localhost:1235/mcp', false],
      ['http://localhost:1234/other', false], ['http://localhost:1234/mcp?x=1', false],
      ['https://localhost:1234/mcp', false], ['http://user@localhost:1234/mcp', false]
    ]) {
      assert.equal(Boolean(undici.request(url, { headers: {} })), allowed, url);
      if (!url.includes('user@')) assert.equal(Boolean(fetch(url)), allowed, url);
    }
  `], { cwd: resolve(__dirname, '../..'), env: { ...process.env, WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL: 'http://127.0.0.1:1234/mcp', WEBCODEX_MCP_CONFORMANCE_TOKEN: 'fixture-token' }, encoding: 'utf8' });
  assert.equal(scope.status, 0, scope.stderr);
  const server = http.createServer((req, res) => {
    if (req.headers.authorization !== 'Bearer fixture-token') res.writeHead(401);
    else if (req.headers.host === 'evil.example.com') res.writeHead(403);
    else res.writeHead(200);
    res.end();
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  try {
    const port = server.address().port;
    const url = `http://127.0.0.1:${port}/mcp`;
    process.env.WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL = url;
    process.env.WEBCODEX_MCP_CONFORMANCE_TOKEN = 'fixture-token';
    require('../mcp_conformance_auth.cjs');
    const { request } = createRequire(resolve(process.env.WEBCODEX_MCP_CONFORMANCE_HARNESS_BUILD, 'package.json'))('undici');
    for (const host of [`127.0.0.1:${port}`, 'evil.example.com']) {
      const response = await request(url, { method: 'POST', headers: { Host: host } });
      assert.equal(response.statusCode, host.startsWith('127.') ? 200 : 403);
      await response.body.dump();
    }
    const alias = await request(`http://localhost:${port}/mcp`, { method: 'POST', headers: { Host: 'evil.example.com' } });
    assert.equal(alias.statusCode, 403);
    await alias.body.dump();
    const other = await request(`${url}/other`, { method: 'POST' });
    assert.equal(other.statusCode, 401);
    await other.body.dump();
  } finally {
    server.close();
    await once(server, 'close');
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
