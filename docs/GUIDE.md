# Switchboard user guide

Switchboard runs many Claude Code agents on one screen, on macOS and Linux. It tells you which one needs you. It also walks you through a pull request review, one step at a time.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/eserilev/switchboard/master/scripts/install.sh | bash
```

`INSTALL.md` has the details.

## The leader key

Keys go to the agent you're in. To send a key to Switchboard, press the **leader** first.

- Linux: `Ctrl+Space`
- macOS: `Ctrl+;` (the installer sets it)

"Leader `n`" means: press the leader, then `n`. Press the second key within 1.5 seconds.

## The screen

- **Left:** the rail. It holds Board, Tasks, Work and Reviews, plus Settings at the bottom.
- **Top:** the header of the screen you're on. The gold button is the main thing to do there.
- **Right:** the details panel. Leader `i` opens and closes it. Less common actions and Close live here.

## Board: your agents

Each agent gets a tile. A tile shows its name, its state and its last lines of output.

| Shape | State | Meaning |
|---|---|---|
| Gold diamond | Needs you | It asks for permission or asks a question |
| Green dot | Your turn | It finished, or asked something in plain text |
| Dashed ring | Working | It's busy |
| Purple square | Limit | Your account hit its usage limit |
| Red triangle | Error | It crashed or hit an error |

A small dot before the name means you haven't looked at it yet.

- **Start an agent:** Leader `n`, pick a repo, pick where it runs, press Enter.
  - **Main checkout** runs in the repo itself.
  - **New worktree** runs in a separate copy of the repo on a new branch, so two agents don't edit the same files.
- **First run in a new folder:** Claude asks if you trust it. Click **Trust** on the tile.
- **Go to the next one that needs you:** Leader `j`.
- **Move between tiles:** Leader `H` `J` `K` `L`.
- **Open a tile big:** press Enter on it, or Leader `x`. Leader `x` closes it again.
- **Answer a permission:** Allow or Deny on the tile.
- **An agent that stopped:** click **Resume** to carry on.
- **Close an agent:** Delete on the focused tile, or **Close pane** in the details panel.
- **Move a tile:** drag it, or Leader `<` / `>`.
- **Editor and shell:** Leader `e` opens nvim next to the agent. Leader `s` opens a shell under it.

If an account hits its limit, click **Switch to…** on the tile. The agent goes on with your second account and keeps its conversation.

## Work: one repo, your own desk

Leader `w`. Pick a repo in the header, then press **Start Claude**. You get nvim on the left, Claude on the top right and a terminal on the bottom right. This agent stays off the board.

## Review a pull request

1. Leader `n` and paste the PR link.
2. Claude splits the PR into small steps. Each says what it does and why.
3. Read the step and its code.
4. Ask Claude about it in the right-hand column. Ask "where is X" or "what should change", and the screen can jump to the code.
5. To comment, hover a line, click **+**, write, then click **Add to review**. Shift-click a second **+** to cover several lines.
6. Press `n` to mark the step done and go to the next one.
7. When you're done, click **Finish review · N**, pick Comment, Approve or Request changes, and send it. Nothing goes to GitHub before that click.

| Key | Action |
|---|---|
| `j` / `k` | Next or previous step |
| `n` | Mark done, next step |
| `f` | Switch between steps and the plain file list |
| `o` | Open the file in nvim |
| `'` | Go back to where you were before a jump |

If the author pushes new commits, the header says **New commits**. Click it, then **Start round N+1**: you see only what changed since your last review.

## Review your agent's work

Click **Review** on the open agent's title bar, or in its details panel. You see what it changed: its commits and any work it hasn't committed yet.

- **New** shows what changed since you last clicked **Mark reviewed**. **All** shows everything.
- Comment with **+**, then click **Send N comments to agent**. The comments land in the agent's prompt. Read them, then press Enter.
- **Base** in the details panel sets what the work is compared with, for example `upstream/main`. Switchboard remembers it for the repo.
- **Update base** downloads the latest version of that branch.

## Tasks

Leader `t`. A simple to-do board with sprints. Press `o` for a new task, keys `1` to `5` set its status, and `s` starts an agent for it.

## Settings

Click the gear at the bottom of the rail to set:
- the folders to look for repos in;
- the sites the review agent may read;
- the sandbox: its mode, and the sites its commands may reach.

The sandbox lets an agent's commands write only in its repo and reach only the sites you list. It is on for review agents by default, since a PR can hide instructions.

Everything else, like the leader key and accounts, is in `~/.config/switchboard/config.toml`. Restart after editing it.

## From the terminal

| Command | Action |
|---|---|
| `sb open .` | Start an agent in this folder |
| `sb open <path> -w <branch>` | Start an agent on a new branch |
| `sb connect add b` | Add a second Claude account. Type `/login` in the pane it opens. |

## Run it on another computer

Put `remote = "me@server"` in your config, with Switchboard installed on both computers. The window runs on your computer, and the agents run on the server over ssh.
