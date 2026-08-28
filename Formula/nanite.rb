class Nanite < Formula
  desc "Manage local repositories in an AI-first workspace"
  homepage "https://github.com/icepuma/nanite"
  version "0.2.0"
  license "MIT"

  depends_on "fzf"
  uses_from_macos "git"

  on_macos do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.2.0/nanite-v0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "0aec4c848f17972783912e19c48715671338a3539aef982695593514952eb6ee"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.2.0/nanite-v0.2.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "62b21e3d5909b730d8d9c5f1e4f7e79836d064baa0b2413140003cf823707ae9"
    end

    on_intel do
      url "https://github.com/icepuma/nanite/releases/download/v0.2.0/nanite-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "e8547a543f608c6fe7c64b617e4a51ecd511486880d7e04409277fb8012157f9"
    end
  end

  def install
    bin.install "nanite"
    doc.install "README.md"
  end

  test do
    assert_match "nanite #{version}", shell_output("#{bin}/nanite --version")
  end
end
