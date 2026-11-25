// -*- mode: Bluespec; -*-
// Pairing, sessions and removal of devices (SPEC 41.4). Each action is one critical
// section of the code: `pairing.rs` (wait, ask, allow, refuse), `devices.rs` (link)
// and `remote.rs` (window, listed, end_unlisted). The QR code of each round has its
// own secret. A key that "saw" a round's code can prove its secret.
//
// The flags of `stepWith` turn one check off at a time; each such step must fail.
module pairing {
  pure val KEYS = Set("phone", "thief")
  pure val ROUNDS = Set(1, 2, 3)
  pure val SESSIONS = Set(1, 2, 3)
  pure val NO_S = { key: "", gen: 0, state: "none", ran: 0 }

  type Session = { key: str, gen: int, state: str, ran: int }

  var s: {
    round: int,              // the rounds of pairing mode so far
    open: bool,              // pairing mode is on
    used: bool,              // this round's secret is used up
    saw: Set[(str, int)],    // who saw which round's code
    asking: str,             // the key that waits for your answer
    listed: Set[str], revoked: Set[str],
    gen: str -> int,
    allowed: Set[str],       // the keys you said Allow to
    proofs: int -> int,      // the proofs taken in each round
    outside: bool,           // a proof came in outside pairing mode
    again: bool,             // a revoked key was listed again
    late: bool,              // a command ran after its key's revoke
    sess: int -> Session,
    nextS: int,
  }

  action init = s' = {
    round: 0, open: false, used: false, saw: Set(), asking: "",
    listed: Set(), revoked: Set(), gen: KEYS.mapBy(_ => 0), allowed: Set(),
    proofs: ROUNDS.mapBy(_ => 0), outside: false, again: false, late: false,
    sess: SESSIONS.mapBy(_ => NO_S), nextS: 1,
  }

  action Open = all {
    not(s.open), s.asking == "", s.round < 3,
    s' = { ...s, round: s.round + 1, open: true, used: false },
  }
  // A key sees the code that is, or was, on the screen.
  action See(k: str): bool = all {
    s.round > 0,
    s' = { ...s, saw: s.saw.union(Set((k, s.round))) },
  }
  // pairing.rs ask: the first frame opens, and the secret is not used up.
  action Prove(k: str, once: bool, mode: bool): bool = all {
    s.round > 0,
    s.saw.contains((k, s.round)),
    if (once) not(s.used) else true,
    if (mode) s.open else true,
    s' = { ...s, used: true, open: false, asking: k,
      outside: s.outside or not(s.open),
      proofs: s.proofs.set(s.round, s.proofs.get(s.round) + 1) },
  }
  action Expire = all { s.open, s' = { ...s, open: false, used: true } }
  // pairing.rs allow: the store refuses a key that is, or was, on the list.
  action Allow(unique: bool): bool = all {
    s.asking != "",
    val k = s.asking
    val refused = unique and s.listed.union(s.revoked).contains(k)
    s' = if (refused) { ...s, asking: "" } else { ...s, asking: "",
      listed: s.listed.union(Set(k)), allowed: s.allowed.union(Set(k)),
      again: s.again or s.revoked.contains(k) },
  }
  action Refuse = all { s.asking != "", s' = { ...s, asking: "" } }

  // devices.rs link: the handshake finds the key on the list.
  action Shake(k: str): bool = all {
    s.nextS <= 3, s.listed.contains(k),
    s' = { ...s, nextS: s.nextS + 1,
      sess: s.sess.set(s.nextS, { key: k, gen: s.gen.get(k), state: "shaken", ran: 0 }) },
  }
  def ok(x: Session): bool = s.listed.contains(x.key) and s.gen.get(x.key) == x.gen
  // remote.rs window: it joins the list of windows; `listed` then looks at the row.
  action Join(i: int): bool = all {
    s.sess.get(i).state == "shaken",
    s' = { ...s, sess: s.sess.set(i, { ...s.sess.get(i), state: if (ok(s.sess.get(i))) "up" else "closed" }) },
  }
  // remote.rs answer: `listed`, then the command.
  action Run(i: int, check: bool): bool = all {
    val x = s.sess.get(i)
    all {
      x.state == "up", x.ran < 2,
      s' = if (check and not(ok(x))) { ...s, sess: s.sess.set(i, { ...x, state: "closed" }) }
        else { ...s, sess: s.sess.set(i, { ...x, ran: x.ran + 1 }), late: s.late or s.revoked.contains(x.key) },
    }
  }
  // store.rs device_revoke.
  action Revoke(k: str): bool = all {
    s.listed.contains(k),
    s' = { ...s, listed: s.listed.exclude(Set(k)), revoked: s.revoked.union(Set(k)),
      gen: s.gen.set(k, s.gen.get(k) + 1) },
  }
  // remote.rs end_unlisted.
  action Sweep = s' = { ...s, sess: s.sess.keys().mapBy(i => {
    val x = s.sess.get(i)
    if (x.state == "up" and not(ok(x))) { ...x, state: "closed" } else x }) }

  action stepWith(once: bool, mode: bool, unique: bool, check: bool): bool = {
    nondet k = KEYS.oneOf()
    nondet i = SESSIONS.oneOf()
    any {
      Open, See(k), Prove(k, once, mode), Expire, Allow(unique), Refuse,
      Shake(k), Join(i), Run(i, check), Revoke(k), Sweep,
    }
  }
  action step = stepWith(true, true, true, true)
  action stepManyUses = stepWith(false, false, true, true)
  action stepNoMode = stepWith(false, false, false, true)
  action stepPairAgain = stepWith(true, true, false, true)
  action stepNoCheck = stepWith(true, true, true, false)

  // P1. No key is on the list without your Allow.
  val OnlyAllowed = s.listed.union(s.revoked).subseteq(s.allowed)
  // P2. Nothing pairs outside pairing mode.
  val OnlyInMode = not(s.outside)
  // P3. A secret proves one key at most.
  val OneUse = ROUNDS.forall(r => s.proofs.get(r) <= 1)
  // P4. A removed key never comes back.
  val NeverAgain = not(s.again)
  // P5. After a remove, no session of that key runs a command, also one whose
  // handshake came first.
  val NothingLate = not(s.late)
  val Always = and { OnlyAllowed, OnlyInMode, OneUse, NeverAgain, NothingLate }

  // Witnesses: each must fail, or the runs miss that path.
  val NoRun = SESSIONS.forall(i => s.sess.get(i).ran == 0)
  val NoRaceClosed = SESSIONS.forall(i => not(s.sess.get(i).state == "closed" and s.sess.get(i).ran == 0))
  val NoReturn = not(s.asking != "" and s.revoked.contains(s.asking))
}
