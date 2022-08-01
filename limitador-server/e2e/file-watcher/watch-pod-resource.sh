#!/bin/bash
POD_TYPE_NAME=`kubectl get pods -l app=limitador -o name`
while true
do
    echo "#######################################################"
    echo "#######################################################"
    echo "## POD: ${POD_TYPE_NAME} ########"
    kubectl describe ${POD_TYPE_NAME}
    sleep 5
done
