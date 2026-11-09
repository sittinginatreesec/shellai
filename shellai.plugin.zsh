# shellai zsh integration. Source this file after placing `shellai` on PATH.

function _shellai_replace_buffer() {
  local request="$1"
  local context="$2"
  local result

  zle -R 'shellai: generating command…'
  if [[ -n "$context" ]]; then
    result="$(command shellai ask --context "$context" -- "$request")"
  else
    result="$(command shellai ask -- "$request")"
  fi
  local exit_status=$?
  if (( exit_status != 0 )); then
    zle -R 'shellai: generation failed (see error above)'
    return $exit_status
  fi

  BUFFER="$result"
  CURSOR=${#BUFFER}
  zle -R
}

function shellai-hotkey() {
  local original="$BUFFER"
  local requirement
  local history_file="${SHELLAI_HISTORY_FILE:-${XDG_STATE_HOME:-$HOME/.local/state}/shellai/history}"
  local history_directory="${history_file:h}"
  local history_size="${SHELLAI_HISTORY_SIZE:-1000}"
  local -i read_status=0

  setopt localoptions extendedglob hist_ignore_all_dups
  [[ "$history_size" == <1-> ]] || history_size=1000

  if ! command mkdir -p -m 700 "$history_directory" 2>/dev/null; then
    zle -R 'shellai: could not create the history directory'
    return 1
  fi

  if ! fc -p -a "$history_file" "$history_size" "$history_size"; then
    zle -R 'shellai: could not load prompt history'
    return 1
  fi

  autoload -Uz read-from-minibuffer
  read-from-minibuffer '🐢 shellai: '
  read_status=$?
  requirement="$REPLY"
  REPLY=''

  if (( read_status == 0 )) && [[ -n "$requirement" ]]; then
    print -s -- "$requirement"
  fi
  fc -P
  command chmod 600 "$history_file" 2>/dev/null

  (( read_status != 0 )) && return $read_status
  [[ -z "$requirement" ]] && return 0
  _shellai_replace_buffer "$requirement" "$original"
}

function shellai-accept-line() {
  if [[ "$BUFFER" == 'ai,' || "$BUFFER" == 'ai,'[[:space:]]* ]]; then
    local request="${BUFFER#ai,}"
    request="${request#"${request%%[![:space:]]*}"}"
    if [[ -z "$request" ]]; then
      zle -R 'usage: ai, <request>'
      return 1
    fi
    _shellai_replace_buffer "$request" ''
  else
    zle .accept-line
  fi
}

# Also useful from scripts; interactive `ai, ...` is intercepted before execution.
function ai,() {
  command shellai ask -- "$@"
}

if [[ -o interactive ]]; then
  zle -N shellai-hotkey
  zle -N shellai-accept-line

  bindkey "${SHELLAI_HOTKEY:-\\ea}" shellai-hotkey
  bindkey -M emacs '^M' shellai-accept-line
  bindkey -M emacs '^J' shellai-accept-line
  bindkey -M viins '^M' shellai-accept-line
  bindkey -M viins '^J' shellai-accept-line
fi

