# Fedora / openSUSE / RHEL spec for memreduct
# Build: rpmbuild -ba packaging/fedora/memreduct.spec  (with Source0 tarball in SOURCES)

Name:           memreduct
Version:        0.2.0
Release:        1%{?dist}
Summary:        Monitor and reclaim Linux memory caches (Mem Reduct port)
License:        GPL-3.0-only
URL:            https://github.com/AkumaNomu/MemReRust
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust

%global _description %{expand:
Rust CLI port of Henry++ Mem Reduct. Reads /proc and sysfs, drives
drop_caches, registers PSI triggers, manages cgroup v2 memory limits, and
reports zram telemetry. Every subcommand supports --json output.}

%description %_description

%prep
%autosetup -n %{name}-%{version}
cargo fetch --locked

%build
cargo build --locked --release

%install
install -Dpm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm0644 README.md %{buildroot}%{_pkgdocdir}/README.md

./target/release/%{name} completions bash > %{name}.bash
./target/release/%{name} completions zsh > _%{name}
./target/release/%{name} completions fish > %{name}.fish
install -Dpm0644 %{name}.bash %{buildroot}%{_datadir}/bash-completion/completions/%{name}
install -Dpm0644 _%{name} %{buildroot}%{_datadir}/zsh/site-functions/_%{name}
install -Dpm0644 %{name}.fish %{buildroot}%{_datadir}/fish/vendor_completions.d/%{name}.fish
install -Dpm0644 packaging/systemd/memreduct-watch.service.example \
    %{buildroot}%{_pkgdocdir}/memreduct-watch.service.example

%check
cargo test --locked --release

%files
%license LICENSE
%doc README.md %{_pkgdocdir}/memreduct-watch.service.example
%{_bindir}/%{name}
%{_datadir}/bash-completion/completions/%{name}
%{_datadir}/zsh/site-functions/_%{name}
%{_datadir}/fish/vendor_completions.d/%{name}.fish

%changelog
* Tue Aug 25 2026 AkumaNomu <akumanomu@proton.me> - 0.2.0-1
- Add compact, slab, oom subcommands; watch cooldown/exec/swap-threshold;
  shell completions; MemFree reporting.
* Sat Aug 16 2026 AkumaNomu <akumanomu@proton.me> - 0.1.0-1
- Initial Linux port: status/clean/watch/pss/grow/zram/reclaim/limit.
