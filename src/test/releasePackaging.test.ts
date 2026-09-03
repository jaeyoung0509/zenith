import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

const repositoryRoot = resolve(import.meta.dirname, '../..');
const temporaryDirectories: string[] = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe('release packaging contracts', () => {
  it('keeps every release manifest synchronized', () => {
    const packageVersion = JSON.parse(
      readFileSync(join(repositoryRoot, 'package.json'), 'utf8'),
    ).version;
    expect(() =>
      execFileSync(
        process.execPath,
        ['scripts/bump_version.cjs', 'check', `v${packageVersion}`],
        {
          cwd: repositoryRoot,
          stdio: 'pipe',
        },
      ),
    ).not.toThrow();
  });

  it('generates a versioned WinGet multi-file manifest for the immutable installer URL', () => {
    const fixtureRoot = mkdtempSync(join(tmpdir(), 'zenith-winget-'));
    temporaryDirectories.push(fixtureRoot);
    const installer = join(fixtureRoot, 'Zenith-windows-x64-setup.exe');
    const installerBytes = Buffer.from('deterministic NSIS fixture');
    writeFileSync(installer, installerBytes);

    execFileSync(
      process.execPath,
      [
        'scripts/generate_winget_manifest.cjs',
        '--version',
        '0.2.0',
        '--installer',
        installer,
        '--output',
        fixtureRoot,
      ],
      { cwd: repositoryRoot, stdio: 'pipe' },
    );

    const manifestRoot = join(
      fixtureRoot,
      'manifests',
      'z',
      'jaeyoung0509',
      'Zenith',
      '0.2.0',
    );
    const versionManifest = readFileSync(join(manifestRoot, 'jaeyoung0509.Zenith.yaml'), 'utf8');
    const installerManifest = readFileSync(
      join(manifestRoot, 'jaeyoung0509.Zenith.installer.yaml'),
      'utf8',
    );
    const localeManifest = readFileSync(
      join(manifestRoot, 'jaeyoung0509.Zenith.locale.en-US.yaml'),
      'utf8',
    );
    const expectedHash = createHash('sha256').update(installerBytes).digest('hex').toUpperCase();

    expect(versionManifest).toContain('ManifestType: version');
    expect(installerManifest).toContain('InstallerType: nullsoft');
    expect(installerManifest).toContain('Scope: user');
    expect(installerManifest).toContain('MinimumOSVersion: 10.0.17763.0');
    expect(installerManifest).toContain('ElevationRequirement: elevationProhibited');
    expect(installerManifest).toContain(
      'InstallerUrl: https://github.com/jaeyoung0509/zenith/releases/download/v0.2.0/Zenith-windows-x64-setup.exe',
    );
    expect(installerManifest).toContain(`InstallerSha256: ${expectedHash}`);
    expect(localeManifest).toContain('License: MIT');
    expect(localeManifest).toContain('ManifestType: defaultLocale');
  });

  it('writes and combines portable LF-only checksum manifests', () => {
    const fixtureRoot = mkdtempSync(join(tmpdir(), 'zenith-checksums-'));
    temporaryDirectories.push(fixtureRoot);
    const macArtifact = join(fixtureRoot, 'Zenith-macos-arm64.dmg');
    const windowsArtifact = join(fixtureRoot, 'Zenith-windows-x64-setup.exe');
    const macManifest = join(fixtureRoot, 'SHA256SUMS-macos-arm64.txt');
    const windowsManifest = join(fixtureRoot, 'SHA256SUMS-windows-x64.txt');
    const combinedManifest = join(fixtureRoot, 'SHA256SUMS.txt');
    writeFileSync(macArtifact, 'mac fixture');
    writeFileSync(windowsArtifact, 'windows fixture');

    for (const [artifact, manifest] of [
      [macArtifact, macManifest],
      [windowsArtifact, windowsManifest],
    ]) {
      execFileSync(
        process.execPath,
        ['scripts/release_checksums.cjs', 'write', '--file', artifact, '--output', manifest],
        { cwd: repositoryRoot, stdio: 'pipe' },
      );
    }

    const windowsChecksum = readFileSync(windowsManifest, 'utf8');
    writeFileSync(windowsManifest, windowsChecksum.replace(/\n/g, '\r\n'));
    execFileSync(
      process.execPath,
      [
        'scripts/release_checksums.cjs',
        'combine',
        '--output',
        combinedManifest,
        macManifest,
        windowsManifest,
      ],
      { cwd: repositoryRoot, stdio: 'pipe' },
    );

    const combined = readFileSync(combinedManifest, 'utf8');
    expect(combined).not.toContain('\r');
    expect(combined).toMatch(/Zenith-macos-arm64\.dmg\n/);
    expect(combined).toMatch(/Zenith-windows-x64-setup\.exe\n$/);
  });

  it('uses Node 24-based artifact actions in CI and release workflows', () => {
    const workflows = ['.github/workflows/ci.yml', '.github/workflows/release.yml']
      .map((workflow) => readFileSync(join(repositoryRoot, workflow), 'utf8'))
      .join('\n');

    expect(workflows).toContain('pnpm/action-setup@v6');
    expect(workflows).toContain('actions/upload-artifact@v7');
    expect(workflows).toContain('actions/download-artifact@v8');
    expect(workflows).not.toMatch(/pnpm\/action-setup@v4/);
    expect(workflows).not.toMatch(/actions\/upload-artifact@v4/);
    expect(workflows).not.toMatch(/actions\/download-artifact@v5/);
  });
});
