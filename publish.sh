#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

success() {
    echo -e "${GREEN}✓${NC} $1"
}

warn() {
    echo -e "${YELLOW}⚠${NC} $1"
}

error() {
    echo -e "${RED}✗${NC} $1"
    exit 1
}

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    error "Not in a git repository"
fi

current_branch=$(git rev-parse --abbrev-ref HEAD)

if [ "$current_branch" != "main" ]; then
  error "You are not on the main branch."
fi

# Check for uncommitted changes
if [[ -n $(git status -s) ]]; then
    warn "You have uncommitted changes:"
    git status -s
    echo
    read -p "Do you want to continue? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        error "Aborted by user"
    fi
fi

# Get current version from Cargo.toml
CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
info "Current version: ${CURRENT_VERSION}"

# Parse current version
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

# Prompt for version bump type
echo
echo "What type of change is this?"
echo "  1) Major (breaking changes)    ${MAJOR}.x.x -> $((MAJOR + 1)).0.0"
echo "  2) Minor (new features)         x.${MINOR}.x -> ${MAJOR}.$((MINOR + 1)).0"
echo "  3) Patch (bug fixes)            x.x.${PATCH} -> ${MAJOR}.${MINOR}.$((PATCH + 1))"
echo
read -p "Enter choice [1-3]: " -n 1 -r BUMP_TYPE
echo

case $BUMP_TYPE in
    1)
        NEW_VERSION="$((MAJOR + 1)).0.0"
        CHANGE_TYPE="major"
        ;;
    2)
        NEW_VERSION="${MAJOR}.$((MINOR + 1)).0"
        CHANGE_TYPE="minor"
        ;;
    3)
        NEW_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))"
        CHANGE_TYPE="patch"
        ;;
    *)
        error "Invalid choice"
        ;;
esac

info "New version will be: ${NEW_VERSION}"
echo

# Confirm with user
read -p "Proceed with version ${NEW_VERSION}? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    error "Aborted by user"
fi

echo
info "Starting publish process..."
echo

# Step 1: Run tests BEFORE updating version
info "Running tests..."
if ! cargo test --all-features; then
    error "Tests failed! Please fix tests before publishing."
fi
success "All tests passed"

# Step 2: Run clippy BEFORE updating version
info "Running clippy..."
if ! cargo clippy --all-features -- -D warnings; then
    warn "Clippy warnings found. Continue anyway? (y/N)"
    read -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        error "Aborted due to clippy warnings"
    fi
fi
success "Clippy checks passed"

# Step 3: Build documentation BEFORE updating version
info "Building documentation..."
if ! cargo doc --no-deps; then
    error "Documentation build failed!"
fi
success "Documentation built successfully"

# Step 4: Update version in Cargo.toml (only after tests pass)
info "Updating version in Cargo.toml..."
sed -i.bak "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" Cargo.toml
rm Cargo.toml.bak
success "Updated Cargo.toml to version ${NEW_VERSION}"

# Step 5: Test package
info "Testing package (dry-run)..."
if ! cargo publish --dry-run --allow-dirty; then
    # Restore old version on failure
    sed -i.bak "s/^version = \".*\"/version = \"${CURRENT_VERSION}\"/" Cargo.toml
    rm Cargo.toml.bak
    error "Package test failed!"
fi
success "Package test passed"

# Step 6: Commit changes
info "Committing changes..."
git add Cargo.toml
git add Cargo.lock 2>/dev/null || true
git commit -m "chore: bump version to ${NEW_VERSION}" || warn "No changes to commit"
success "Changes committed"

# Step 7: Create git tag
info "Creating git tag v${NEW_VERSION}..."
if git tag -a "v${NEW_VERSION}" -m "Release version ${NEW_VERSION}"; then
    success "Tag created: v${NEW_VERSION}"
else
    error "Failed to create tag (does it already exist?)"
fi

# Step 8: Push to remote
info "Pushing to remote..."
read -p "Push commits and tags to origin? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    git push origin main || git push origin master || warn "Failed to push to main/master branch"
    git push origin "v${NEW_VERSION}"
    success "Pushed to remote"
else
    warn "Skipped pushing to remote (don't forget to push manually!)"
fi

# Step 9: Publish to crates.io
echo
echo "═══════════════════════════════════════════════════════"
echo "  Ready to publish to crates.io!"
echo "═══════════════════════════════════════════════════════"
echo
read -p "Publish to crates.io now? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    info "Publishing to crates.io..."
    if cargo publish; then
        success "Successfully published to crates.io!"
        echo
        echo "═══════════════════════════════════════════════════════"
        echo -e "  ${GREEN}🎉 Version ${NEW_VERSION} published successfully!${NC}"
        echo "═══════════════════════════════════════════════════════"
        echo
        echo "Next steps:"
        echo "  1. Check: https://crates.io/crates/tapaculo"
        echo "  2. Wait for docs: https://docs.rs/tapaculo"
        echo "  3. Create GitHub release: https://github.com/AlexTrebs/tapaculo/releases"
        echo
    else
        error "Failed to publish to crates.io"
    fi
else
    warn "Skipped publishing to crates.io"
    echo
    echo "To publish manually later, run:"
    echo "  cargo publish"
fi

# Step 10: Offer to create GitHub release
echo
read -p "Open GitHub releases page to create a release? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    REPO_URL=$(git remote get-url origin | sed 's/\.git$//')
    RELEASE_URL="${REPO_URL}/releases/new?tag=v${NEW_VERSION}"

    if command -v xdg-open &> /dev/null; then
        xdg-open "$RELEASE_URL"
    elif command -v open &> /dev/null; then
        open "$RELEASE_URL"
    else
        echo "Please visit: $RELEASE_URL"
    fi
fi

echo
success "Done! 🚀"
