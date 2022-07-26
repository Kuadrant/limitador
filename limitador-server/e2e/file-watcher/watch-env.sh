#!/bin/bash
ip=$(kubectl get nodes -lkubernetes.io/hostname!=kind-control-plane -ojsonpath='{.items[*].status.addresses[?(@.type=="InternalIP")].address}')
port=$(kubectl get service limitador -o jsonpath='{.spec.ports[?(@.name=="http")].nodePort}')
POD_TYPE_NAME=`kubectl get pods -l app=limitador -o name`
while true
do
    echo "#######################################################"
    echo "##########Configmap content ###########################"
    kubectl get configmap limitador-config -o yaml | yq e -
    echo "##########limits.yaml content inside the pod ##########"
    kubectl exec $(kubectl get pods -l app=limitador -o name) -- cat /tmp/limitador/limits.yaml
    echo "##########HTTP endpoints limits ##########"
    curl "http://${ip}:${port}/limits/test" 2>/dev/null | yq e -
    sleep 5
done
