const box = document.getElementById("box");
box.addEventListener("submit", async (e) => {
  e.preventDefault();
  const btn = document.getElementById("go"), err = document.getElementById("err");
  btn.disabled = true; err.textContent = "";
  try {
    const r = await fetch("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password: document.getElementById("pw").value }),
    });
    if (r.ok) { location.reload(); return; }
    err.textContent = "access denied";
  } catch {
    err.textContent = "connection error";
  }
  btn.disabled = false;
  box.classList.remove("shake"); void box.offsetWidth; box.classList.add("shake");
});
