class Nanite < Formula
  desc "Manage local repositories in an AI-first workspace"
  homepage "https://github.com/icepuma/nanite"
  version "0.1.11"
  license "MIT"

  depends_on "fzf"
  uses_from_macos "git"

  on_macos do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.11/nanite-v0.1.11-aarch64-apple-darwin.tar.gz"
      sha256 "5d7513ee7cf5425bd711aa16ed088d72e0c645f9462a1c4edf2215b3220008c0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.11/nanite-v0.1.11-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0d6e68e71677834892112c4e1fc63f6bc235e1853b262b7d9bbccbc57275a6b9"
    end

    on_intel do
      url "https://github.com/icepuma/nanite/releases/download/v0.1.11/nanite-v0.1.11-x86_64-unknown-linux-musl.tar.gz"
      sha256 "2a3e09ce1118dae798ace883cd66fc370a667d6ffa57b6688c85096138073eda"
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
