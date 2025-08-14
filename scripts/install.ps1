param(
    [switch]$Force
)

$REPO_OWNER = "rhythmcache"
$REPO_NAME = "payload-dumper-rust"
$GITHUB_API_URL = "https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"

$installDir = "$env:USERPROFILE\.extra"
$binPath = "$installDir\payload_dumper.exe"
function Get-YesNoInput {
    param([string]$Prompt)
    
    while ($true) {
        $response = Read-Host "$Prompt (y/n)"
        switch -Regex ($response.ToLower()) {
            '^(y|yes)$' { return $true }
            '^(n|no)$' { return $false }
            default { Write-Host "Please answer yes (y) or no (n)." -ForegroundColor Yellow }
        }
    }
}

function Extract-Version {
    param([string]$VersionString)
    
    if ($VersionString -match '(\d+\.\d+\.\d+)') {
        return $matches[1]
    }
    return $null
}

$existingPath = Get-Command payload_dumper -ErrorAction SilentlyContinue
if ($existingPath -or (Test-Path $binPath)) {
    if ($existingPath) {
        $installedPath = $existingPath.Source
    } else {
        $installedPath = $binPath
    }
    
    Write-Host "INFO: payload_dumper is already installed at: $installedPath" -ForegroundColor Cyan
    
    # Get current version
    try {
        $currentVersionOutput = & $installedPath --version 2>$null
        if ($LASTEXITCODE -eq 0 -and $currentVersionOutput) {
            $currentVersion = Extract-Version $currentVersionOutput
            Write-Host "INFO: Current version: $currentVersion" -ForegroundColor Cyan
            
            Write-Host "INFO: Fetching latest release information..." -ForegroundColor Cyan
            try {
                $releaseInfo = Invoke-RestMethod -Uri $GITHUB_API_URL -ErrorAction Stop
                $releaseTag = $releaseInfo.tag_name
                $latestVersion = Extract-Version $releaseTag
                Write-Host "INFO: Latest version: $latestVersion" -ForegroundColor Cyan
                
                if ($currentVersion -eq $latestVersion) {
                    Write-Host "INFO: You already have the latest version installed." -ForegroundColor Green
                    if (-not $Force) {
                        $reinstall = Get-YesNoInput "Do you still want to reinstall it?"
                        if (-not $reinstall) {
                            Write-Host "INFO: Installation cancelled." -ForegroundColor Yellow
                            exit 0
                        }
                    }
                    Write-Host "INFO: Proceeding with reinstallation..." -ForegroundColor Cyan
                } else {
                    Write-Host "INFO: A newer version is available!" -ForegroundColor Green
                    Write-Host "INFO: Updating from $currentVersion to $latestVersion..." -ForegroundColor Cyan
                    
                    $existingDir = Split-Path $installedPath -Parent
                    $canWriteToExisting = $false
                    
                    try {
                        $testFile = Join-Path $existingDir "test_write_permissions.tmp"
                        [System.IO.File]::WriteAllText($testFile, "test")
                        Remove-Item $testFile -Force
                        $canWriteToExisting = $true
                    } catch {
                        $canWriteToExisting = $false
                    }
                    
                    if ($canWriteToExisting) {
                        Write-Host "INFO: Removing old version..." -ForegroundColor Cyan
                        try {
                            Remove-Item $installedPath -Force
                            Write-Host "SUCCESS: Old version removed successfully" -ForegroundColor Green
                            $installDir = $existingDir
                            $binPath = $installedPath
                            $updateMode = $true
                        } catch {
                            Write-Host "WARNING: Failed to remove old version. You may need elevated permissions." -ForegroundColor Yellow
                            Write-Host "INFO: Will install to default location instead." -ForegroundColor Cyan
                            $updateMode = $false
                        }
                    } else {
                        Write-Host "WARNING: No write permission to $existingDir" -ForegroundColor Yellow
                        Write-Host "INFO: Will install to default location instead." -ForegroundColor Cyan
                        $updateMode = $false
                    }
                }
            } catch {
                Write-Host "WARNING: Failed to fetch release information. Cannot compare versions." -ForegroundColor Yellow
                if (-not $Force) {
                    $proceed = Get-YesNoInput "Do you still want to proceed with installation?"
                    if (-not $proceed) {
                        Write-Host "INFO: Installation cancelled." -ForegroundColor Yellow
                        exit 0
                    }
                }
                Write-Host "INFO: Proceeding with installation..." -ForegroundColor Cyan
            }
        } else {
            Write-Host "WARNING: Could not determine current version (--version command failed)" -ForegroundColor Yellow
            if (-not $Force) {
                $proceed = Get-YesNoInput "Do you want to proceed with installation anyway?"
                if (-not $proceed) {
                    Write-Host "INFO: Installation cancelled." -ForegroundColor Yellow
                    exit 0
                }
            }
            Write-Host "INFO: Proceeding with installation..." -ForegroundColor Cyan
        }
    } catch {
        Write-Host "WARNING: Error checking current version: $($_.Exception.Message)" -ForegroundColor Yellow
        if (-not $Force) {
            $proceed = Get-YesNoInput "Do you want to proceed with installation anyway?"
            if (-not $proceed) {
                Write-Host "INFO: Installation cancelled." -ForegroundColor Yellow
                exit 0
            }
        }
        Write-Host "INFO: Proceeding with installation..." -ForegroundColor Cyan
    }
}

Write-Host "INFO: Checking system architecture..." -NoNewline
$arch = $env:PROCESSOR_ARCHITECTURE.ToUpper()

function Get-AssetPattern {
    param($architecture)
    
    switch -Regex ($architecture.ToUpper()) {
        'AMD64|X64' { return 'x86_64' }
        'X86|I386' { return 'i686' }
        'ARM64|AARCH64' { return 'aarch64' }
        default { return '' }
    }
}

$assetPattern = Get-AssetPattern $arch

# Fallback architecture detection if primary method fails
if ([string]::IsNullOrEmpty($assetPattern)) {
    try {
        $dotNetArch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToUpper()
        $assetPattern = Get-AssetPattern $dotNetArch
    }
    catch {
    }
}

Write-Host " Windows ($arch)" -ForegroundColor Green
Write-Host "INFO: Detected architecture pattern: $assetPattern" -ForegroundColor Cyan

if ([string]::IsNullOrEmpty($assetPattern)) {
    Write-Host "ERROR: Unsupported architecture: $arch" -ForegroundColor Red
    exit 1
}

Start-Sleep -Milliseconds 500

# Fetch release info
if (-not $releaseInfo) {
    Write-Host "INFO: Fetching latest release information..." -ForegroundColor Cyan
    Start-Sleep -Milliseconds 500

    try {
        $releaseInfo = Invoke-RestMethod -Uri $GITHUB_API_URL -ErrorAction Stop
    }
    catch {
        Write-Host "ERROR: Failed to fetch release information: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }

    $releaseTag = $releaseInfo.tag_name
}

Write-Host "INFO: Latest release: $releaseTag" -ForegroundColor Cyan

Write-Host "INFO: Looking for Windows release matching architecture: $arch ($assetPattern)" -ForegroundColor Cyan
Start-Sleep -Milliseconds 500

$matchingAsset = $null
foreach ($asset in $releaseInfo.assets) {
    if ($asset.name -imatch "windows") {
        if ($asset.name -imatch $assetPattern) {
            $matchingAsset = $asset
            break
        }
    }
}

if ($null -eq $matchingAsset) {
    Write-Host "ERROR: No matching Windows release found for architecture: $arch" -ForegroundColor Red
    Write-Host "INFO: Available assets:" -ForegroundColor Yellow
    foreach ($asset in $releaseInfo.assets) {
        Write-Host "    $($asset.name)" -ForegroundColor Gray
    }
    exit 1
}

Write-Host "SUCCESS: Found matching release: $($matchingAsset.name)" -ForegroundColor Green
Write-Host "INFO: Download URL: $($matchingAsset.browser_download_url)" -ForegroundColor Cyan

$tempDir = [System.IO.Path]::GetTempPath() + [System.Guid]::NewGuid().ToString()
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
$zipFile = Join-Path $tempDir $matchingAsset.name

if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

Write-Host "INFO: Downloading release archive..." -ForegroundColor Cyan
Start-Sleep -Milliseconds 500

try {
    Invoke-WebRequest -Uri $matchingAsset.browser_download_url -OutFile $zipFile -ErrorAction Stop
    Write-Host "SUCCESS: Download completed" -ForegroundColor Green
}
catch {
    Write-Host "ERROR: Download failed: $($_.Exception.Message)" -ForegroundColor Red
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

Write-Host "INFO: Extracting archive..." -ForegroundColor Cyan
Start-Sleep -Milliseconds 500

try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zipFile, $tempDir)
    Write-Host "SUCCESS: Archive extracted successfully" -ForegroundColor Green
}
catch {
    Write-Host "ERROR: Failed to extract archive: $($_.Exception.Message)" -ForegroundColor Red
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

$binaryFile = Get-ChildItem -Path $tempDir -Recurse -File | Where-Object { 
    $_.Name -imatch "payload.dumper" -and $_.Extension -eq ".exe" 
} | Select-Object -First 1

if ($null -eq $binaryFile) {
    $binaryFile = Get-ChildItem -Path $tempDir -Recurse -File -Filter "*.exe" | Select-Object -First 1
}

if ($null -eq $binaryFile) {
    Write-Host "ERROR: No executable file found in the extracted archive" -ForegroundColor Red
    Write-Host "INFO: Contents of extracted archive:" -ForegroundColor Yellow
    Get-ChildItem -Path $tempDir -Recurse -File | ForEach-Object { Write-Host "    $($_.Name)" -ForegroundColor Gray }
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

Write-Host "SUCCESS: Found binary: $($binaryFile.Name)" -ForegroundColor Green
Write-Host "INFO: Installing to $binPath" -ForegroundColor Cyan

try {
    Copy-Item -Path $binaryFile.FullName -Destination $binPath -Force
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    
    Write-Host "INFO: Verifying the binary..." -ForegroundColor Cyan
    Start-Sleep -Milliseconds 500
    
    $testResult = & $binPath --help 2>$null
    if ($LASTEXITCODE -eq 0 -or $testResult) {
        if ($updateMode) {
            Write-Host "SUCCESS: Successfully updated payload_dumper to $releaseTag" -ForegroundColor Green
            Write-Host "INFO: Updated executable: $binPath" -ForegroundColor Cyan
        } else {
            Write-Host "SUCCESS: Installed package 'payload_dumper $releaseTag' (executable '$binPath')" -ForegroundColor Green
            
            $currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
            if ($currentPath -notlike "*$installDir*") {
                Write-Host ""
                $addToPath = Get-YesNoInput "Do you want to add $installDir to your PATH environment variable?"
                
                if ($addToPath) {
                    try {
                        $newPath = $currentPath + ";" + $installDir
                        [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
                        Write-Host "SUCCESS: Added to PATH. Please restart your terminal or run the following command:" -ForegroundColor Green
                        Write-Host "         `$env:PATH += ';$installDir'" -ForegroundColor Yellow
                    }
                    catch {
                        Write-Host "WARNING: Failed to add to PATH automatically. Please add manually:" -ForegroundColor Yellow
                        Write-Host "         Add '$installDir' to your PATH environment variable" -ForegroundColor Yellow
                    }
                }
                else {
                    Write-Host "INFO: You can manually add the following to your PATH:" -ForegroundColor Yellow
                    Write-Host "      $installDir" -ForegroundColor Yellow
                }
            }
            else {
                Write-Host "INFO: Directory already in PATH" -ForegroundColor Green
            }
        }
        
        Write-Host ""
        Write-Host "SUCCESS: Installation completed successfully!" -ForegroundColor Green
    }
    else {
        Write-Host "ERROR: Something went wrong. The binary may not be compatible." -ForegroundColor Red
        Write-Host "INFO: Cleaning up..." -ForegroundColor Yellow
        Remove-Item -Path $binPath -Force -ErrorAction SilentlyContinue
        exit 1
    }
}
catch {
    Write-Host "ERROR: Failed to install binary: $($_.Exception.Message)" -ForegroundColor Red
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}