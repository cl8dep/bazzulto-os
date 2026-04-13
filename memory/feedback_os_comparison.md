---
name: OS comparison in answers
description: User wants every technical answer to include how Linux, macOS, Windows, and FreeBSD handle the same concept
type: feedback
---

Always include a comparison across major production OSes (Linux, macOS/XNU, Windows NT, FreeBSD) when answering questions about OS design, kernel behavior, or system internals.

**Why:** User is building Bazzulto OS and wants to learn how professional OSes approach each problem to inform their own design decisions.

**How to apply:** For any question about OS internals (PIDs, syscalls, scheduling, filesystem, memory, etc.), provide a table or breakdown showing how each major OS handles it, then relate back to Bazzulto's current state.
