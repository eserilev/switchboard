#!/usr/bin/env bash
# S1: log the raw input of StopFailure and quota Notification hooks.
# Add it to the hooks for StopFailure and Notification. The next real usage limit
# then shows which fields a limit carries.
mkdir -p ~/.switchboard
{ printf '%s ' "$(date -Is)"; cat; echo; } >>~/.switchboard/s1.log
