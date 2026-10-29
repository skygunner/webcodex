const assert = require('node:assert/strict');
const http = require('node:http');
const { createRequire } = require('node:module');
const { once } = require('node:events');

async function main() {
  const server = http.createServer((req, res) => {
    if (req.headers.authorization !== 'Bearer fixture-test-token') res.writeHead(401);
    else if (req.headers.host !== `127.0.0.1:${server.address().port}`) res.writeHead(403);
    else res.writeHead(200);
    res.end();
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const url = `http://127.0.0.1:${server.address().port}/mcp`;
  process.env.WEBCODEX_MCP_CONFORMANCE_FIXTURE_URL = url;
  process.env.WEBCODEX_MCP_CONFORMANCE_TOKEN = 'fixture-test-token';
  require('../mcp_conformance_auth.cjs');
  const { request } = createRequire(`${process.env.WEBCODEX_MCP_CONFORMANCE_HARNESS_BUILD}/package.json`)('undici');
  try {
    const valid = await request(url, { method: 'POST', headers: { Host: `127.0.0.1:${server.address().port}` } });
    assert.equal(valid.statusCode, 200);
    await valid.body.dump();
    const forbidden = await request(url, { method: 'POST', headers: { Host: 'evil.example.com' } });
    assert.equal(forbidden.statusCode, 403);
    await forbidden.body.dump();
    const other = await request(`${url}/other`, { method: 'POST' });
    assert.equal(other.statusCode, 401);
    await other.body.dump();
  } finally {
    server.close();
    await once(server, 'close');
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
