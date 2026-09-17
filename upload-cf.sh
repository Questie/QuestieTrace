#!/bin/sh

LATEST_GIT_TAG="$1"
CHANGELOG=$(jq --slurp --raw-input '.' < "CHANGELOG.md")

if echo "$LATEST_GIT_TAG" | grep -q "^.*-b.*$"; then
  RELEASE_TYPE="beta"
else
  RELEASE_TYPE="release"
fi

echo "Uploading $RELEASE_TYPE $LATEST_GIT_TAG to CurseForge"

#### CurseForge Upload
# Docs: https://support.curseforge.com/en/support/solutions/articles/9000197321-curseforge-upload-api

# The order of the "gameVersions" below is: Classic Era, Forever, TBC, Wrath (3.80.1), MoP
CF_METADATA=$(cat <<-EOF
{
    "displayName": "$LATEST_GIT_TAG",
    "releaseType": "$RELEASE_TYPE",
    "changelog": $CHANGELOG,
    "changelogType": "markdown",
    "gameVersions": [16630, 17053, 16533, 16785, 16168],
    "relations": {
        "projects": [
            {"slug": "LibStub", "type": "embeddedLibrary"},
            {"slug": "LibDeflate", "type": "embeddedLibrary"}
        ]
    }
}
EOF
)

response=$(curl -sS \
    -o response.txt \
    -w "%{http_code}" \
    -H "X-API-TOKEN: $CF_API_TOKEN" \
    -F "metadata=$CF_METADATA" \
    -F "file=@releases/$LATEST_GIT_TAG/QuestieTrace-$LATEST_GIT_TAG.zip" \
    "https://wow.curseforge.com/api/projects/1695789/upload-file")

http_status=$(echo "$response" | tail -n1)

if [ "$http_status" -eq 200 ]; then
  echo "CurseForge upload successful"
else
  echo "CurseForge upload failed, HTTP-code: $http_status"
  cat response.txt
  exit 1
fi
