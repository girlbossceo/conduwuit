#!/bin/sh
set -eux

# --------------------------------------------------------------------- #
#                                                                       #
#  Configures docker buildx to use a remote server for arm building.    #
#  Expects $SSH_PRIVATE_KEY to be a valid ssh ed25519 private key with  #
#  access to the server $ARM_SERVER_USER@$ARM_SERVER_IP                 # 
#                                                                       #
#  This is expected to only be used in the official CI/CD pipeline!     #
#                                                                       #
#  Requirements: openssh-client, docker buildx                          #
#  Inspired by:  https://depot.dev/blog/building-arm-containers         #
#                                                                       #
# --------------------------------------------------------------------- #

cat "$BUILD_SERVER_SSH_PRIVATE_KEY" | ssh-add -

# Test server connections:
ssh "$ARM_SERVER_USER@$ARM_SERVER_IP" "uname -a"
ssh "$AMD_SERVER_USER@$AMD_SERVER_IP" "uname -a"

# Connect remote arm64 server for all arm builds:
docker buildx create \
    --name "multi" \
    --driver "docker-container" \
    --platform "linux/arm64,linux/arm/v7" \
    "ssh://$ARM_SERVER_USER@$ARM_SERVER_IP"

# Connect remote amd64 server for adm64 builds:
docker buildx create --append \
    --name "multi" \
    --driver "docker-container" \
    --platform "linux/amd64" \
    "ssh://$AMD_SERVER_USER@$AMD_SERVER_IP"

docker buildx use multi
