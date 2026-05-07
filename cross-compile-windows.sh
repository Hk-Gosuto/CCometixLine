#!/bin/bash

echo "🔧 Setting up cross-compilation for Windows on macOS..."

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    echo "❌ Cargo not found. Please install Rust from https://rustup.rs/"
    exit 1
fi

echo "✅ Rust version: $(cargo --version)"

# Add Windows target
echo "📦 Adding Windows x86_64 target..."
rustup target add x86_64-pc-windows-gnu

# Check if mingw-w64 is installed
if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    echo "📦 Installing mingw-w64 via Homebrew..."
    if command -v brew &> /dev/null; then
        brew install mingw-w64
    else
        echo "❌ Homebrew not found. Please install mingw-w64 manually:"
        echo "   brew install mingw-w64"
        echo "   or install Homebrew first: https://brew.sh/"
        exit 1
    fi
fi

echo "✅ mingw-w64 found: $(x86_64-w64-mingw32-gcc --version | head -n1)"

# Configure cargo for cross-compilation
echo "⚙️  Configuring cargo for Windows cross-compilation..."
mkdir -p .cargo
cat > .cargo/config.toml << 'EOF'
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
ar = "x86_64-w64-mingw32-ar"
EOF

echo "🔨 Building Windows version..."
cargo build --release --target x86_64-pc-windows-gnu

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Windows build successful!"
    echo "📁 Binary location: target/x86_64-pc-windows-gnu/release/ccometixline.exe"
    echo "📊 Binary size: $(du -h target/x86_64-pc-windows-gnu/release/ccometixline.exe | cut -f1)"
    
    # Create a Windows distribution folder
    echo "📦 Creating Windows distribution..."
    mkdir -p dist/windows
    cp target/x86_64-pc-windows-gnu/release/ccometixline.exe dist/windows/
    
    # Create a simple test script for Windows
    cat > dist/windows/test.bat << 'EOF'
@echo off
echo Testing CCometixLine...
echo {"model":{"id":"claude-4-sonnet","display_name":"Sonnet 4"},"workspace":{"current_dir":"C:\\Projects\\CCometixLine"},"transcript_path":"C:\\Projects\\session.jsonl"} | ccometixline.exe
pause
EOF
    
    echo "✅ Windows distribution created in dist/windows/"
    echo ""
    echo "📋 Files included:"
    ls -la dist/windows/
    
    echo ""
    echo "🚀 To use on Windows:"
    echo "   1. Copy the dist/windows/ folder to your Windows machine"
    echo "   2. Run test.bat to verify it works"
    echo "   3. Add ccometixline.exe to your PATH"
else
    echo "❌ Windows build failed!"
    exit 1
fi