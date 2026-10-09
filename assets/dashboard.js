const $ = id => document.getElementById(id);

function fmtHash(hs) {
  const units = ["H/s", "KH/s", "MH/s", "GH/s", "TH/s", "PH/s"];
  let i = 0;
  while (hs >= 1000 && i < units.length - 1) { hs /= 1000; i++; }
  return [hs.toFixed(2), units[i]];
}
function fmtBig(n) {
  if (n >= 1e15) return (n / 1e15).toFixed(2) + " P";
  if (n >= 1e12) return (n / 1e12).toFixed(2) + " T";
  if (n >= 1e9)  return (n / 1e9).toFixed(2) + " G";
  if (n >= 1e6)  return (n / 1e6).toFixed(2) + " M";
  return Math.round(n).toLocaleString("en-US");
}
function fmtInt(n) { return Math.round(n).toLocaleString("en-US"); }
function fmtUptime(s) {
  if (s == null) return "—";
  const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60);
  return (d ? d + "d " : "") + h + "h " + m + "m";
}
// "1 in N" odds words, honestly
function fmtOdds(myHs, difficulty) {
  if (!myHs || !difficulty) return null;
  const netHs = difficulty * 2 ** 32 / 600;
  const oneIn = netHs / myHs;
  const names = [[1e12, "trillion"], [1e9, "billion"], [1e6, "million"], [1e3, "thousand"]];
  for (const [v, name] of names)
    if (oneIn >= v) return `1 in ${(oneIn / v).toFixed(1)} ${name}`;
  return `1 in ${fmtInt(oneIn)}`;
}

async function tick() {
  let r;
  try {
    r = await fetch("/api/stats", { signal: AbortSignal.timeout(9000) });
  } catch { $("grid").classList.add("stale"); return; }
  if (r.status === 401) { location.reload(); return; }
  if (!r.ok) { $("grid").classList.add("stale"); return; }
  const d = await r.json();
  $("grid").classList.remove("stale");

  const running = d.miner.running;
  $("led").className = "led " + (running ? "on" : "off");
  $("status").textContent = running ? "MINING" : "OFFLINE";
  $("uptime").textContent = fmtUptime(d.miner.uptime_seconds);
  $("threads").textContent = `${d.miner.threads} of ${d.miner.cores} (${d.miner.power}%)`;
  $("restarts").textContent = d.miner.restarts;
  $("pool").textContent = d.miner.pool_url.replace("stratum+tcp://", "");
  $("worker").textContent = d.miner.worker.split(".").pop() || d.miner.worker;

  const [hv, hu] = fmtHash(d.pool.hashrate_10m);
  $("hashrate").textContent = hv;
  $("hashunit").textContent = hu;
  $("hash1h").textContent = fmtHash(d.pool.hashrate_1h).join(" ");
  $("shares").textContent = fmtInt(d.pool.accepted_shares);
  $("bestdiff").textContent = fmtBig(d.pool.best_difficulty);
  $("workers").textContent = d.pool.workers;

  if (d.network.btc_usd) $("price").textContent = "$" + fmtInt(d.network.btc_usd);
  if (d.network.block_height) $("height").textContent = fmtInt(d.network.block_height);
  if (d.network.difficulty) $("netdiff").textContent = fmtBig(d.network.difficulty);
  $("lastpool").textContent = d.network.last_block_pool || "—";
  $("halvblocks").textContent = fmtInt(d.network.halving_blocks_left);
  $("halvdays").textContent = "~ " + fmtInt(d.network.halving_days_left);

  const wallet = d.miner.worker.split(".")[0];
  $("poolLink").href = "https://web.public-pool.io/#/app/" + wallet;

  // Prefer the live 10-min window; fall back to the 1h average so a fresh
  // session (empty 10-min window) still shows meaningful odds
  const oddsHash = d.pool.hashrate_10m || d.pool.hashrate_1h;
  const odds = fmtOdds(oddsHash, d.network.difficulty);
  $("odds").textContent = odds || "waiting for pool data…";
  if (odds) {
    const window = d.pool.hashrate_10m ? "current hashrate" : "1h average";
    $("oddsNote").textContent = `per block, at your ${window} — that's the lottery you're playing`;
  }
}

tick();
setInterval(tick, 10_000);
