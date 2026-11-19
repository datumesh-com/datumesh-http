# Datumesh relay: licence check (build2)

Method: pages fetched with curl, User-Agent `datumesh-licence-check/1.0`, no login, no forms. Raw bytes are in `raw/<slug>.html` (headers in `raw/<slug>.hdr`). Fetch instants are the HTTP `Date` response header (UTC), 2026-11-19. Quotes are verbatim from the fetched pages. Verdict RELAY means the licence clearly allows relaying values unchanged, publicly, by a third party, including commercially. Anything else is SKIP.

Summary

| # | Source | Verdict |
|---|--------|---------|
| 1 | ECB euro foreign exchange reference rates (+ Data Portal) | RELAY |
| 2 | ECB euro short-term rate (EUR STR) | RELAY |
| 3 | ECB key policy rates | RELAY |
| 4 | World Bank Pink Sheet | SKIP |
| 5 | FAO ASIS | SKIP |
| 6 | CHIRPS rainfall | RELAY |
| 7 | US Treasury daily par yield curve | RELAY (statutory basis, see caveat) |
| 8 | Bank of England Bank Rate | RELAY |

---

## ECB common terms (apply to sources 1, 2, 3)

- Page: https://www.ecb.europa.eu/services/using-our-site/disclaimer/html/index.en.html (requested as https://www.ecb.europa.eu/services/disclaimer/html/index.en.html, redirected)
- Fetched: 2026-11-19T05:33:50Z; slug `ecb-copyright`; sha256 `5c467e3b05d8e84acda31b952c7db2ced96c18bad746119039cb03da96fbf6db`
- The ECB Data Portal (https://data.ecb.europa.eu/, slug `ecb-dp-about`, fetched 2026-11-19T05:34:57Z, sha256 `b2d7814315c9ce7ade8e0f49686bdac7e0e5749c0c936ff0637abc121f8b6a8e`) has no separate terms page; its footer links to "Disclaimer & Copyright" = the page above. (https://data.ecb.europa.eu/help/terms-and-conditions returned 404.)

Verbatim:

- Copyright: "Copyright © for the entire content of this website: European Central Bank, Frankfurt am Main, Germany."
- Reuse: "Subject to the exception below, users of this website may make free use of the information obtained directly from it subject to the following conditions:" / "When such information is distributed or reproduced, it must appear accurately and the ECB must be cited as the source."
- Sold documents: "Where the information is incorporated in documents that are sold (regardless of the medium), the natural or legal person publishing the information must inform buyers, both before they pay any subscription or fee and each time they access the information taken from this website, that the information may be obtained free of charge through this website."
- Modification: "If the information is modified by the user (e.g. by seasonal adjustment of statistical data or calculation of growth rates) this must be stated explicitly."
- Linking: "When linking to this website from business sites or for promotional purposes, this website must load into the browser's entire window (i.e. it must not appear within another website's frame)."
- Exception (does not cover data series): "any reproduction, publication or reprint, in whole or in part, of documents that bear the name of their authors, such as ECB Working Papers and ECB Occasional Papers ... is permitted only with the explicit prior written authorisation of the ECB or the authors."
- Disclaimer: "While the euro short-term rate (€STR), the euro foreign exchange reference rates and other price information published on this website are based on sources which the ECB considers to be reliable, the ECB accepts no responsibility or liability for any loss, damage, expense or claim incurred in connection with any decision made or action or inaction on the part of any party in reliance upon such publication."
- Authenticity: "Only ECB legal instruments as published in the Official Journal of the European Union and other ECB documentation and/or data as published by the ECB are deemed to be authentic."

Commercial use is not restricted ("free use"); the only fee-related condition is the notice to buyers when information is in sold documents (relevant if datumesh.com ever charges).

---

## 1. ECB euro foreign exchange reference rates

- Licence page: ECB common terms above; rates page https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.en.html (fetched 2026-11-19T05:33:51Z, slug `ecb-fx`, sha256 `1f47e003872222c058195a74dc44cd67db054413c9fb66df697a77a9eedf1b7c`).
- Rates page text: "The reference rates are usually updated at around 16:00 CET" and "The reference rates are published for information purposes only. Using the rates for transaction purposes is strongly discouraged."
- Reuse / modification / commercial / attribution: see ECB common terms (free use; accurate; cite ECB; state modifications; sold-documents notice).
- Data URLs: https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml (HEAD 200, text/xml, Last-Modified 2026-11-18T14:56:15Z); also eurofxref-hist-90d.xml, eurofxref-hist.xml, eurofxref-hist.zip, eurofxref-sdmx.xml; or SDMX API https://data-api.ecb.europa.eu/service/data/EXR/... (format=csvdata).
- Frequency: each TARGET working day, about 16:00 CET.
- Verdict: RELAY. The grant is free use with conditions we can meet (cite ECB, unchanged, show disclaimer "for information purposes only"). Any derived value (e.g. inverse or cross rate) must be labelled as modified.

## 2. ECB euro short-term rate (EUR STR)

- Licence pages: ECB common terms plus the specific €STR page https://www.ecb.europa.eu/stats/financial_markets_and_interest_rates/euro_short-term_rate/html/index.en.html (fetched 2026-11-19T05:33:51Z, slug `ecb-estr`, sha256 `70005d3f5f48c7f41d1f7e8b8029efc0bc284cbf855cccfe2a09334fb12ae551`).
- Verbatim, licensing: "The ECB does not charge for the €STR or license its use."
- Verbatim, €STR disclaimer: "The euro short-term rate, and any re-publication thereof (the “rate”) and all other related data and information, including the compounded euro short-term rate average rates and the compounded euro short-term rate index, (the “information”) are published by the European Central Bank (ECB) for public information purposes only." / "The ECB and the Eurosystem national central banks, in so far as they contribute to the euro short-term rate determination process, shall not be liable for any error or inaccuracy in the rate or the information, for any delay in updating the rate or the information, nor shall they be liable or responsible for any reliance on or use of or inability to use the rate or the information in any financial instrument, contract, trade or any other commercial activity or investment decision."
- The general disclaimer states it is "without prejudice to the euro short-term rate (€STR) disclaimer". The general copyright conditions (cite ECB, accurate, modifications stated) apply to the website content, so apply them too.
- Data URLs: https://data-api.ecb.europa.eu/service/data/EST/B.EU000A2X2A25.WT?format=csvdata (HEAD 200, text/csv, Last-Modified 2026-11-18T07:05:26Z; use lastNObservations=N for a small sample).
- Frequency: each TARGET business day, for the previous TARGET business day's trading (published by 08:00 CET per the ECB; the page states "published on each TARGET2 business day based on transactions conducted and settled on the previous TARGET2 business day").
- Verdict: RELAY. The ECB explicitly says it does not license or charge for use, the rate is published for public information, and commercial use is expressly contemplated in the disclaimer (as a non-liability matter). Carry the disclaimer and cite ECB.

## 3. ECB key policy rates (MRO, deposit facility, marginal lending)

- Licence: ECB common terms. Page: https://www.ecb.europa.eu/stats/policy_and_exchange_rates/key_ecb_interest_rates/html/index.en.html (fetched 2026-11-19T05:34:58Z, slug `ecb-rates`, sha256 `66c45dcbebef6d3d0ab53891cb91d0a5775635939961c0b45102be8808bd3fb8`).
- Reuse / modification / commercial / attribution: see ECB common terms.
- Data URLs: ECB Data Portal, dataflow FM, e.g. https://data-api.ecb.europa.eu/service/data/FM/B.U2.EUR.4F.KR.MRR_FR.LEV?format=csvdata (MRO fixed rate; HEAD 200, text/csv, Last-Modified 2026-09-10T14:22:33Z); deposit facility key `B.U2.EUR.4F.KR.DFR.LEV`, marginal lending `B.U2.EUR.4F.KR.MLFR.LEV` (series keys from ECB convention; only the MRO key was sampled).
- Frequency: changes only at Governing Council rate decisions (about every 6 weeks); series updated the same day.
- Verdict: RELAY, same reasoning as source 1.

---

## 4. World Bank Commodity Price Data ("Pink Sheet")

- Pages:
  - https://www.worldbank.org/en/research/commodity-markets (fetched 2026-11-19T05:33:52Z, slug `wb-pink`, sha256 `bfd6dbac15b86a15665ff709c13d3238152862ae0ef449fbb0d79df5c7c0c20b`). Under "Using this Data" it only links "Summary terms of use", "Terms of use for Datasets" and "Data Access and Licensing"; it states no licence for the Pink Sheet itself.
  - https://datacatalog.worldbank.org/public-licenses (fetched 2026-11-19T05:34:17Z, slug `wb-publiclic`, sha256 `f7106582fe705d8ea31c2bd7ef726636942b870104a88d882563b16bcece3910`).
  - "Terms of use for Datasets" (https://www.worldbank.org/en/about/legal/terms-of-use-for-datasets) redirects to the generic site terms https://www.worldbank.org/ext/en/legal/terms-conditions (fetched 2026-11-19T05:33:52Z, slug `wb-terms`, sha256 `c534067add5e892b5f8a51a4f8a84a7d1f5d6ec51310e1ad3b26f8ec3410de19`). "Summary terms of use" (data.worldbank.org) returned 403 to curl (Cloudflare challenge, slug `wb-summary`), so it could not be read.
- Verbatim, data catalog: "The World Bank Group makes data publicly available according to open data standards and licenses datasets under the Creative Commons Attribution 4.0 International license (CC-BY 4.0). Many datasets are available under other licenses. They are labeled accordingly" / "The Creative Commons Attribution 4.0 International license allows users to copy, modify and distribute data in any format for any purpose, including commercial use. Users are only obligated to give appropriate credit (attribution) and indicate if they have made any changes, including translations. CC-BY 4.0, with the additional terms below, is the default license for all Datasets produced by the World Bank itself and distributed as open data."
- Verbatim, generic site terms (the page the dataset-terms URL resolves to): "The content available on this platform is provided for informational and non-commercial purposes only, unless otherwise stated." / "For the remainder of the Materials, you may make non-commercial uses thereof, but you may not make any derivative work or commercial use, including without limitation reselling them, charging to access them, charging to redistribute them, or charging for derivative works based on them, without the prior written consent of the relevant member institution(s)." / "For use of the APIs in connection with some of the Materials (such as Datasets listed in The World Bank [Group] Data Catalog), The World Bank Group has specific terms of use, all of which are hereby incorporated by reference."
- Data URL: https://thedocs.worldbank.org/en/doc/74e8be41ceb20fa0da750cda2f6b9e4e-0050012026/related/CMO-Historical-Data-Monthly.xlsx (HEAD 200, xlsx, Last-Modified 2026-11-03T19:30:42Z; the path contains a per-release hash and changes each release, so it must be rediscovered from the page). XLSX only, no CSV/API found.
- Frequency: monthly, early in the month (Nov 2026 release 3 Nov).
- Verdict: SKIP. The default for World Bank-produced datasets is CC-BY 4.0, but the Pink Sheet page itself does not state a licence, the dataset-specific terms page could not be reached (it resolves to generic site terms that forbid commercial use and redistribution), I did not find a Data Catalog entry labelling the Pink Sheet, and some price series come from third-party sources. Not clear enough. Re-check the Pink Sheet's Data Catalog entry or the notes sheet in the XLSX, or ask the World Bank to confirm CC-BY 4.0, then it could become RELAY with CC-BY attribution.

## 5. FAO Agricultural Stress Index System (ASIS)

- Pages:
  - https://www.fao.org/contact-us/terms/en/ (fetched 2026-11-19T05:33:52Z, slug `fao-terms`, sha256 `724a8b680d45f6c43298bac2d2d89b5adb6bae508350062cfe0405fcee55ff08`)
  - https://www.fao.org/contact-us/terms/db-terms-of-use/en/ (Statistical Database Terms of Use; fetched 2026-11-19T05:34:56Z, slug `fao-stat-terms`, sha256 `bb826bd021941263ae4f12c0fc3c5419f8a02f95ffd3d523d2abc4fafcddeae3`)
  - ASIS page https://www.fao.org/giews/earthobservation/asis/index_1.jsp?lang=en (fetched 2026-11-19T05:33:52Z, slug `fao-asis`, sha256 `a29bb04576fab2afb509e7f380103159e80d2b202d0026e5ffd5c0662b32fbc9`) contains no licence or terms statement.
- Verbatim, general web content terms: "All other content on the FAO website (except where otherwise indicated), may be copied, printed and downloaded for private study, research and teaching purposes, and for use in non-commercial products or services, provided that appropriate acknowledgement of FAO as the source and copyright holder is given and that FAO's endorsement of users' views, products or services is not stated or implied in any way." / "All requests for translation and adaptation rights, and for resale and other commercial use rights should be addressed to" the FAO licence request form.
- Verbatim, statistical databases: "Unless specified otherwise in their metadata or webpage, all datasets disseminated through FAO corporate statistical databases (see examples in Annex 1) are licensed under the Creative Commons Attribution-4.0 International licence (CC BY 4.0)". Prohibited use: "Datasets shall not be used for or in conjunction with the promotion of a commercial enterprise and/or its product(s) or services (s), and/or in any way that suggests that FAO endorses any specific company, products or services." Attribution format: "FAO. [YYYY (year of last update)]. [Name of database: Name of dataset OR Name of database]. [Accessed on [DD Month YYYY]]. [URL] Licence: CC-BY-4.0." Third party: "FAO corporate statistical databases may include data provided by third parties which may not be redistributed or reused without the consent of the original data provider".
- ASIS does not appear in Annex 1 (search of the fetched page for ASIS, Agricultural Stress, Earth Observation and GIEWS found nothing).
- Data URL (sample HEAD only): https://www.fao.org/giews/earthobservation/asis/data/as/ASI/DATA/ASI_AnnualSummary_Season1_data.csv (200, text/csv, 444,638 bytes, Last-Modified 2026-07-07). Maps/rasters via the ASIS site (open_WMS.jsp).
- Frequency: dekadal (every 10 days) for the indices; annual summaries per season.
- Verdict: SKIP. ASIS is a GIEWS Earth Observation product, not shown to be in the FAO statistical databases CC BY regime, and the ASIS pages carry no licence. The default FAO web terms permit only non-commercial use and require a licence request for commercial use. Even under the database regime there is a restriction on use "in conjunction with the promotion of a commercial enterprise" that conflicts with a commercial relay. Ask FAO for written confirmation.

## 6. CHIRPS rainfall (Climate Hazards Center, UCSB)

- Pages:
  - https://www.chc.ucsb.edu/data/chirps (CHIRPS v2; fetched 2026-11-19T05:33:52Z, slug `chc`, sha256 `a4311d63a3ad74640571b3b5a24e6492e422251fbf687a5e17b1153e3f375a31`)
  - https://www.chc.ucsb.edu/data/chirps3 (CHIRPS v3; fetched 2026-11-19T05:36:28Z, slug `chc3`, sha256 `ffb74626a0047bb5e05e2a44534cb318fe8011eadf88379568d9e7ef2ee97cef`)
- Verbatim, v2: "To the extent possible under the law, Pete Peterson has waived all copyright and related or neighboring rights to CHIRPS. CHIRPS data is in the public domain as registered with Creative Commons. This work is published from the United States."
- Verbatim, v3: "CHIRPS3 is in the public domain, as registered with Creative Commons, and is licensed under a Creative Commons Attribution 4.0 International License. To the extent possible under the law, the Climate Hazards Center has waived all copyright and related or neighboring rights to CHIRPS3. This work is published from the United States."
- Modification / commercial: not restricted (CC0-style waiver; CC BY 4.0 allows modification and commercial use). No required notice text is given on the pages. The v3 page says CC BY 4.0, so credit CHC and cite the dataset (Funk et al. 2015, https://www.nature.com/articles/sdata201566, for v2).
- Data URLs: v3 https://data.chc.ucsb.edu/products/CHIRPS/v3.0/ (directory listing 200; daily/pentad/dekad/monthly/annual; GeoTIFF, NetCDF, BIL, COG; the server returned 429 on rapid repeat requests, so throttle). The v2.0 root https://data.chc.ucsb.edu/products/CHIRPS-2.0/ (linked from the page) returned 404 at fetch time, so v2.0 paths are unverified.
- Frequency: v3 preliminary available 2 days after each pentad (on the 2nd, 7th, 12th, 17th, 22nd, 27th of each month); final product later; daily, pentad, dekad, monthly products derived.
- Verdict: RELAY. Public-domain waiver and/or CC BY 4.0; both allow unchanged public commercial redistribution. Uncertainty: the two statements differ (CC0-style waiver vs CC BY 4.0), so keep an attribution line, which satisfies either. These are gridded rasters, large; plan for a derived/subset representation (which would be modification and must be labelled).

## 7. US Treasury daily par yield curve rates

- Pages:
  - https://home.treasury.gov/policy-issues/financing-the-government/interest-rate-statistics (fetched 2026-11-19T05:33:52Z, slug `treas`, sha256 `eca0f47aee6a8ac41a04e434a69384ea2db75d6893938573ea1bf558d204b761`)
  - https://home.treasury.gov/subfooter/privacy-policy (includes "Legal Disclaimers"; fetched 2026-11-19T05:35:57Z, slug `treas-privacy`, sha256 `32c8c8db1b23fa8689a0c13f32f6fed688ec4e28c629873361cbba797db1e4ee`)
  - https://home.treasury.gov/subfooter/site-policies-and-notices (fetched 2026-11-19T05:35:57Z, slug `treas-policies`, sha256 `ffa5aa8f5caa3e06825f97f3fd29d628f857817556dba9469b42e5c3c2128083`): contains no copyright or reuse statement.
  - 17 U.S.C. 105, https://www.law.cornell.edu/uscode/text/17/105 (fetched 2026-11-19T05:36:14Z, slug `us105`, sha256 `e0f425493dd80314c4777e99d29a229f1e72ffedd433891c958dec0500fed6d5`)
- Verbatim, statute: "Copyright protection under this title is not available for any work of the United States Government".
- Verbatim, Treasury: "The United States Government (including the Department of the Treasury) makes no warranty, express or implied, including the warranties of merchantability and fitness for a particular purpose and assumes no legal liability or responsibility for the accuracy, completeness, or usefulness of any information, apparatus, product, or process described or depicted on this web site and does not represent that its use would not infringe privately owned rights." / "The Department of the Treasury does not endorse any commercial product, service, process, or enterprise."
- Treasury page describing the data: "This par yield curve ... is based on the closing market bid prices on the most recently auctioned Treasury securities in the over-the-counter market."
- Reuse / modification / commercial / attribution: no restriction stated by Treasury; none required.
- Data URLs: XML https://home.treasury.gov/resource-center/data-chart-center/interest-rates/pages/xml?data=daily_treasury_yield_curve&field_tdr_date_value_month=202611 (HEAD 200, text/xml); CSV https://home.treasury.gov/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/2026/all?type=daily_treasury_yield_curve&field_tdr_date_value=2026&page&_format=csv (HEAD 200, text/csv). The page carries a "Developer Notice on changes to the XML data feeds".
- Frequency: each business day (published after the close, afternoon US Eastern time).
- Verdict: RELAY (caveat). There is no explicit licence page: the basis is the statute (no copyright in US Government works) and the absence of any use restriction on the site. The only Treasury notices are warranty/endorsement disclaimers. Do not imply Treasury endorsement. Uncertain: Treasury has no on-page "public domain" sentence; if you require an explicit grant on the page, treat it as SKIP.

## 8. Bank of England Bank Rate

- Pages: https://www.bankofengland.co.uk/legal (fetched 2026-11-19T05:33:55Z, slug `boe-terms`, sha256 `b7b9b9c13e01980e91a77c632820a47305ecaec8b11f3551eeb4913299545bfd`); Open Government Licence v3.0, https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/ (fetched 2026-11-19T05:35:49Z, slug `ogl3`, sha256 `f5b2b9f2af63647cde889fa6c3508f5705925295b74912c68caa28dd37e64aa5`). The Statistical Interactive Database landing page (slug `boe-sdb`) and its Help.asp page (slug `boe-db-help`) were also fetched; they carry no separate licence.
- Verbatim, BoE: "The information made available via the Database is the copyright of the Governor and Company of the Bank, unless otherwise stated. Reproduction of data in the Database is subject to the terms of the UK Open Government Licence, allowing and encouraging free and flexible data reuse." / "Please note that selected exchange rate data and series are excluded from this licence as they are reproduced by the Bank under licence from third parties." / for SONIA only: "SONIA and/or SONIA Compounded Index data licensed under the Open Government Licence v3.0 and copyright the Governor and Company of the Bank of England. The trade marks “Bank of England” and “SONIA” are registered trade marks of the Bank of England." / The Bank "gives no assurance that information currently published will continue to be published in the future".
- Verbatim, OGL v3.0: "You are free to: copy, publish, distribute and transmit the Information; adapt the Information; exploit the Information commercially and non-commercially for example, by combining it with other Information, or by including it in your own product or application." / "You must (where you do any of the above): acknowledge the source of the Information in your product or application by including or linking to any attribution statement specified by the Information Provider(s) and, where possible, provide a link to this licence;" / "If the Information Provider does not provide a specific attribution statement, you must use the following: Contains public sector information licensed under the Open Government Licence v3.0."
- Data URL: https://www.bankofengland.co.uk/boeapps/database/_iadb-fromshowcolumns.asp?csv.x=yes&Datefrom=01/Jan/2026&Dateto=now&SeriesCodes=IUDBEDR&CSVF=TN&UsingCodes=Y&VPD=Y&VFD=N (HEAD 200, application/csv, 4,001 bytes; IUDBEDR = official Bank Rate, daily series). The 4,001 byte size suggests a full-year sample; no auth.
- Frequency: series is daily, value changes only at MPC decisions (about 8 per year).
- Verdict: RELAY. Bank Rate is the Bank's own data, not one of the third-party exchange-rate exclusions. Required attribution: the BoE gives a specific statement only for SONIA, so use the OGL default plus BoE copyright: "Contains public sector information licensed under the Open Government Licence v3.0." Uncertain: the BoE page does not name the OGL version (links to "UK Open Government Licence"); v3.0 is assumed. Bank warns it may stop publishing.
