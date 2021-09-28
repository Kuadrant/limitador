#!/bin/bash
set -euxo pipefail

# This script deploys the environment described in
# limitador-server/kubernetes/README.md in a local cluster using kind
# (https://github.com/kubernetes-sigs/kind)

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
CLUSTER_NAME=limitador
LIMITADOR_NAMESPACE=default

kind delete cluster --name ${CLUSTER_NAME}
kind create cluster --name ${CLUSTER_NAME} --config "${SCRIPT_DIR}"/kind-cluster.yaml

# Deploy Redis
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/redis-service.yaml
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/redis-statefulset.yaml

# Deploy Limitador
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/limitador-config-configmap.yaml
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/limitador-service.yaml
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/limitador-deployment.yaml

# Deploy Kuard. In kind we don't use the proxy protocol, so we need to disable
# the option in the Envoy config
sed 's/use_proxy_proto: true/use_proxy_proto: false/g' "${SCRIPT_DIR}"/../kubernetes/kuard-envoy-config-configmap.yaml | kubectl -n ${LIMITADOR_NAMESPACE} apply -f -
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/kuard-service.yaml
kubectl -n ${LIMITADOR_NAMESPACE} apply -f "${SCRIPT_DIR}"/../kubernetes/kuard-deployment.yaml

# Wait for everything to be ready
for deployment in $(kubectl -n ${LIMITADOR_NAMESPACE} get deploy -oname)
do
  kubectl -n ${LIMITADOR_NAMESPACE} wait --timeout=300s --for=condition=Available "$deployment"
done

# Get Kuard host and port
ips=$(kubectl get nodes -lkubernetes.io/hostname!=kind-control-plane -ojsonpath='{.items[*].status.addresses[?(@.type=="InternalIP")].address}')
port=$(kubectl -n ${LIMITADOR_NAMESPACE} get service kuard -ojsonpath='{.spec.ports[?(@.name=="envoy-http")].nodePort}')

echo "You can connect to the Kuard service limited by Limitador at ${ips[0]}:${port}"
