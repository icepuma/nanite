class Nanite < Formula
  desc "Manage local repositories in an AI-first workspace"
  homepage "https://github.com/icepuma/nanite"
  version "0.1.7"
  license "MIT"

  depends_on "fzf"
  uses_from_macos "git"

  on_macos do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.7/nanite-v0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "b06d0e435a8e0c4df8d9651b2a6da5a00f03ef0a6690977cd27ec7bad8b8c6a3"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.7/nanite-v0.1.7-aarch64-unknown-linux-musl.tar.gz"
      sha256 "752a194f644955dc656e2c27df2e92a56fdd78fe7cc3e5a40fc6d62d4d926158"
    end

    on_intel do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.7/nanite-v0.1.7-x86_64-unknown-linux-musl.tar.gz"
      sha256 "ad3badca90e4385696ddd46b4d082642a6b13d244e9a5c7abb0c652befa90392"
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
