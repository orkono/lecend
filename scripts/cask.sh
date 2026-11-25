#!/usr/bin/env bash
# Prints the Homebrew cask for a release: cask.sh <version without v> <SHA256SUMS file>
set -euo pipefail
version="$1"
sums="$2"
sha() {
  local line
  line="$(grep -- "-$1.tar.gz\$" "$sums")" || { echo "no checksum for $1 in $sums" >&2; exit 1; }
  echo "${line%% *}"
}
block() {
  cat <<EOF
    on_$1 do
      sha256 "$(sha "$2")"
      url "https://github.com/orkono/lecend/releases/download/v#{version}/lecend-v#{version}-$2.tar.gz"
      binary "lecend-v#{version}-$2/lecend"
    end
EOF
}
cat <<EOF
cask "lecend" do
  version "$version"

  on_macos do
$(block arm aarch64-apple-darwin)
$(block intel x86_64-apple-darwin)
  end
  on_linux do
$(block arm aarch64-unknown-linux-gnu)
$(block intel x86_64-unknown-linux-gnu)
  end

  name "lecend"
  desc "Local glossary of the short labels in a long Claude Code session"
  homepage "https://github.com/orkono/lecend"

  postflight do
    if OS.mac?
      system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{staged_path}"]
    end
  end

  zap :not_required

  caveats <<~EOS
    Next, in Claude Code:
      claude plugin marketplace add orkono/lecend
      claude plugin install lecend@lecend
  EOS
end
EOF
