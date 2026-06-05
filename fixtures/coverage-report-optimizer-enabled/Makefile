.PHONY: TargetContractBasicOnce
TargetContractBasicOnce:
	@genhtml reports/TargetContractBasicOnce.info -o coverage/TargetContractBasicOnce --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractBasicOnce/

.PHONY: TargetContractBasicTwice
TargetContractBasicTwice:
	@genhtml reports/TargetContractBasicTwice.info -o coverage/TargetContractBasicTwice --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractBasicTwice/
