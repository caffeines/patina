# Homebrew formula for Patina.
#
# It builds Patina from source on the Mac it's installed on, so it needs no
# Apple Developer ID or notarization: Gatekeeper checks apps downloaded from
# the internet, and this one is compiled locally.
#
#   brew tap caffeines/patina https://github.com/caffeines/patina
#   brew install caffeines/patina/patina           # the latest release
#   brew install --HEAD caffeines/patina/patina    # the main branch
class Patina < Formula
  desc "Containers for your Mac, with its own Linux VM and Docker Engine"
  homepage "https://github.com/caffeines/patina"
  url "https://github.com/caffeines/patina.git", tag: "v0.1.0"
  head "https://github.com/caffeines/patina.git", branch: "main"

  depends_on "rust" => :build
  depends_on arch: :arm64
  depends_on macos: :ventura

  def install
    system "cargo", "install", *std_cargo_args
    # Patina.app: the same binary, with its icon, signed (ad hoc) with the
    # virtualization entitlement the engine's VM needs.
    with_env(PATINA_BIN: bin/"patina") { system "scripts/bundle-macos.sh" }
    prefix.install "target/Patina.app"
  end

  def caveats
    <<~EOS
      Start Patina from a terminal:
        patina

      Or add Patina.app to Applications:
        ln -sf #{opt_prefix}/Patina.app /Applications/Patina.app

      The first launch sets up the engine: it downloads Debian and installs
      Docker Engine in Patina's own VM.
    EOS
  end

  test do
    output = shell_output("#{bin}/patina --version")
    assert_match "patina", output
    assert_match version.to_s, output unless version.head?
    system bin/"patina", "--write-icon", testpath/"icon.png"
    assert_path_exists testpath/"icon.png"
    assert_match "com.apple.security.virtualization",
                 shell_output("codesign -d --entitlements - --xml #{prefix}/Patina.app")
  end
end
