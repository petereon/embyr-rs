/* embyr console — Auth gate (login + MFA + recovery) */
const AuthGate = (function () {
  const { useState, useRef, useEffect } = React;
  const Icon = window.Icon;

  function CodeInput({ length = 6, value, onChange }) {
    const refs = useRef([]);
    function set(i, v) {
      v = v.replace(/\D/g, "").slice(-1);
      const arr = value.split("");
      arr[i] = v; const next = arr.join("").slice(0, length);
      onChange(next);
      if (v && i < length - 1) refs.current[i + 1]?.focus();
    }
    function onKey(i, e) {
      if (e.key === "Backspace" && !value[i] && i > 0) refs.current[i - 1]?.focus();
    }
    return (
      <div className="row" style={{ gap: 8, justifyContent: "center" }}>
        {Array.from({ length }).map((_, i) => (
          <input key={i} ref={(el) => refs.current[i] = el} value={value[i] || ""} inputMode="numeric"
            onChange={(e) => set(i, e.target.value)} onKeyDown={(e) => onKey(i, e)}
            onPaste={(e) => { const t = e.clipboardData.getData("text").replace(/\D/g, "").slice(0, length); if (t) { onChange(t); e.preventDefault(); refs.current[Math.min(t.length, length - 1)]?.focus(); } }}
            style={{ width: 46, height: 54, textAlign: "center", fontSize: 22, fontWeight: 600, fontFamily: "var(--font-mono)",
              background: "var(--bg-elev)", border: "1px solid var(--border-2)", borderRadius: 10, color: "var(--text)", outline: "none" }}
            onFocus={(e) => { e.target.style.borderColor = "var(--accent)"; e.target.style.boxShadow = "var(--ring)"; }}
            onBlur={(e) => { e.target.style.borderColor = "var(--border-2)"; e.target.style.boxShadow = "none"; }} />
        ))}
      </div>
    );
  }

  function Auth({ onAuthed }) {
    const [step, setStep] = useState("login"); // login | mfa | recovery
    const [email, setEmail] = useState("you@personal.dev");
    const [pw, setPw] = useState("");
    const [showPw, setShowPw] = useState(false);
    const [err, setErr] = useState("");
    const [code, setCode] = useState("");
    const [mfaMode, setMfaMode] = useState("totp"); // totp | email
    const [recovery, setRecovery] = useState("");
    const [busy, setBusy] = useState(false);

    function login(e) {
      e?.preventDefault();
      if (!pw) { setErr("Enter your password."); return; }
      setErr(""); setBusy(true);
      setTimeout(() => { setBusy(false); setStep("mfa"); setCode(""); }, 500);
    }
    function verify() {
      if (code.length < 6) { setErr("Enter the 6-digit code."); return; }
      setErr(""); setBusy(true);
      setTimeout(() => { setBusy(false); onAuthed(); }, 500);
    }
    function verifyRecovery() {
      if (recovery.replace(/[^a-z0-9]/gi, "").length < 8) { setErr("Enter a full recovery code."); return; }
      setBusy(true); setTimeout(() => onAuthed(), 500);
    }

    return (
      <div style={{ display: "grid", gridTemplateColumns: "1.05fr 1fr", height: "100%", background: "var(--bg)" }}>
        {/* brand panel */}
        <div style={{ position: "relative", overflow: "hidden", borderRight: "1px solid var(--border)",
          background: "radial-gradient(120% 120% at 15% 10%, color-mix(in oklab, var(--accent) 16%, var(--bg)) 0%, var(--bg) 55%)",
          display: "flex", flexDirection: "column", justifyContent: "space-between", padding: "44px 48px" }}>
          <div className="row" style={{ gap: 11 }}>
            <img src="assets/embyr-mark.svg" width="30" height="30" alt="" />
            <span style={{ fontSize: 19, fontWeight: 600, letterSpacing: "-0.03em" }}>embyr</span>
          </div>
          <div>
            <div style={{ fontSize: 34, fontWeight: 600, letterSpacing: "-0.03em", lineHeight: 1.12, maxWidth: 440 }}>
              The realtime database, <span style={{ color: "var(--accent)" }}>without the lock-in.</span>
            </div>
            <p style={{ color: "var(--text-2)", fontSize: 15, marginTop: 16, maxWidth: 380, lineHeight: 1.55 }}>
              Firestore-compatible APIs over your own Postgres. Manage databases, keys, and access from one console.
            </p>
            <div className="row" style={{ gap: 20, marginTop: 28, color: "var(--text-3)", fontSize: 12.5 }}>
              <span className="row" style={{ gap: 7 }}><Icon name="zap" size={14} style={{ color: "var(--accent)" }} />Sub-10ms reads</span>
              <span className="row" style={{ gap: 7 }}><Icon name="shield" size={14} style={{ color: "var(--accent)" }} />mTLS + MFA</span>
              <span className="row" style={{ gap: 7 }}><Icon name="server" size={14} style={{ color: "var(--accent)" }} />Self-hostable</span>
            </div>
          </div>
          <span style={{ fontSize: 12, color: "var(--text-3)" }}>© 2026 embyr-rs · admin console</span>
        </div>

        {/* form panel */}
        <div style={{ display: "grid", placeItems: "center", padding: 32 }}>
          <div style={{ width: "100%", maxWidth: 360 }} className="fade-in" key={step}>
            {step === "login" && (
              <form onSubmit={login}>
                <h1 style={{ fontSize: 23, fontWeight: 620, letterSpacing: "-0.02em", margin: "0 0 6px" }}>Sign in</h1>
                <p style={{ color: "var(--text-2)", fontSize: 13.5, margin: "0 0 24px" }}>Welcome back. Continue to your console.</p>
                <div className="col" style={{ gap: 9, marginBottom: 18 }}>
                  <window.Button variant="default" block onClick={() => onAuthed()} type="button"><Icon name="globe" size={16} />Continue with Google</window.Button>
                  <window.Button variant="default" block onClick={() => onAuthed()} type="button"><Icon name="external" size={16} />Continue with GitHub</window.Button>
                </div>
                <div className="row" style={{ gap: 12, margin: "4px 0 18px", color: "var(--text-3)", fontSize: 11.5 }}>
                  <div className="divider" style={{ flex: 1 }} /> OR <div className="divider" style={{ flex: 1 }} />
                </div>
                <div className="col" style={{ gap: 14 }}>
                  <window.Field label="Email"><window.Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} /></window.Field>
                  <window.Field label="Password" error={err}>
                    <div className="input-group">
                      <input className={`input ${err ? "input-err" : ""}`} type={showPw ? "text" : "password"} value={pw} autoFocus placeholder="••••••••" onChange={(e) => setPw(e.target.value)} style={{ paddingRight: 38 }} />
                      <button type="button" className="iconbtn" style={{ position: "absolute", right: 3, width: 30, height: 30 }} onClick={() => setShowPw((s) => !s)}><Icon name={showPw ? "eyeOff" : "eye"} size={15} /></button>
                    </div>
                  </window.Field>
                  <window.Button variant="primary" block size="lg" type="submit" disabled={busy}>{busy ? "Signing in…" : "Continue"}</window.Button>
                </div>
                <p style={{ textAlign: "center", fontSize: 12.5, color: "var(--text-3)", marginTop: 18 }}>Protected by mandatory MFA</p>
              </form>
            )}

            {step === "mfa" && mfaMode === "totp" && (
              <div style={{ textAlign: "center" }}>
                <div style={{ width: 46, height: 46, borderRadius: 12, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", margin: "0 auto 16px" }}><Icon name="shield" size={22} /></div>
                <h1 style={{ fontSize: 21, fontWeight: 620, margin: "0 0 6px" }}>Two-factor authentication</h1>
                <p style={{ color: "var(--text-2)", fontSize: 13.5, margin: "0 0 24px" }}>Enter the 6-digit code from your authenticator app.</p>
                <CodeInput value={code} onChange={setCode} />
                {err && <div className="field-err" style={{ justifyContent: "center", marginTop: 12 }}><Icon name="alert" size={13} />{err}</div>}
                <window.Button variant="primary" block size="lg" style={{ marginTop: 22 }} onClick={verify} disabled={busy}>{busy ? "Verifying…" : "Verify"}</window.Button>
                <div className="row" style={{ justifyContent: "center", gap: 14, marginTop: 18, fontSize: 12.5 }}>
                  <button className="btn btn-ghost btn-sm" onClick={() => { setMfaMode("email"); setCode(""); setErr(""); }}><Icon name="mail" size={14} />Email me a code</button>
                  <button className="btn btn-ghost btn-sm" onClick={() => { setStep("recovery"); setErr(""); }}><Icon name="key" size={14} />Recovery code</button>
                </div>
              </div>
            )}

            {step === "mfa" && mfaMode === "email" && (
              <div style={{ textAlign: "center" }}>
                <div style={{ width: 46, height: 46, borderRadius: 12, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", margin: "0 auto 16px" }}><Icon name="mail" size={22} /></div>
                <h1 style={{ fontSize: 21, fontWeight: 620, margin: "0 0 6px" }}>Check your email</h1>
                <p style={{ color: "var(--text-2)", fontSize: 13.5, margin: "0 0 24px" }}>We sent a 6-digit code to <b style={{ color: "var(--text)" }}>{email}</b>. It expires in 10 minutes.</p>
                <CodeInput value={code} onChange={setCode} />
                {err && <div className="field-err" style={{ justifyContent: "center", marginTop: 12 }}><Icon name="alert" size={13} />{err}</div>}
                <window.Button variant="primary" block size="lg" style={{ marginTop: 22 }} onClick={verify} disabled={busy}>{busy ? "Verifying…" : "Verify"}</window.Button>
                <button className="btn btn-ghost btn-sm" style={{ marginTop: 16 }} onClick={() => { setMfaMode("totp"); setCode(""); setErr(""); }}><Icon name="arrowLeft" size={14} />Use authenticator instead</button>
              </div>
            )}

            {step === "recovery" && (
              <div style={{ textAlign: "center" }}>
                <div style={{ width: 46, height: 46, borderRadius: 12, background: "var(--accent-soft)", color: "var(--accent)", display: "grid", placeItems: "center", margin: "0 auto 16px" }}><Icon name="key" size={22} /></div>
                <h1 style={{ fontSize: 21, fontWeight: 620, margin: "0 0 6px" }}>Use a recovery code</h1>
                <p style={{ color: "var(--text-2)", fontSize: 13.5, margin: "0 0 22px" }}>Enter one of your single-use recovery codes.</p>
                <window.Input mono value={recovery} placeholder="xxxx-xxxx" style={{ textAlign: "center", fontSize: 16, height: 48, letterSpacing: "0.08em" }} onChange={(e) => setRecovery(e.target.value)} />
                {err && <div className="field-err" style={{ justifyContent: "center", marginTop: 12 }}><Icon name="alert" size={13} />{err}</div>}
                <window.Button variant="primary" block size="lg" style={{ marginTop: 20 }} onClick={verifyRecovery} disabled={busy}>{busy ? "Verifying…" : "Sign in"}</window.Button>
                <button className="btn btn-ghost btn-sm" style={{ marginTop: 16 }} onClick={() => { setStep("mfa"); setErr(""); }}><Icon name="arrowLeft" size={14} />Back</button>
              </div>
            )}
          </div>
        </div>
      </div>
    );
  }
  return Auth;
})();
window.AuthGate = AuthGate;
