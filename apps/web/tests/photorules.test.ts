import { describe, expect, it } from "vitest";
import {
  authorName,
  captionYears,
  commonsPage,
  exifMismatch,
  headquartersNamed,
  licenceAllowed,
  licenceUrl,
  originalSource,
  permissionTerms,
  requestNotCovered,
  stripFlickrPlace,
  takenYear,
} from "../scripts/photo-rules.mjs";

// The photo build's gates (scripts/build-photos.mjs applies exactly these functions).

describe("authorName", () => {
  it("keeps a plain name, from Commons HTML", () => {
    expect(authorName('<a href="//commons.wikimedia.org/wiki/User:Coolcaesar">Coolcaesar</a>')).toBe("Coolcaesar");
    expect(authorName("Radosław Drożdżewski (Zwiadowca21)")).toBe("Radosław Drożdżewski (Zwiadowca21)");
    expect(authorName("User:proshob")).toBe("proshob");
    expect(authorName("Bobak Ha&#39;Eri")).toBe("Bobak Ha'Eri");
    expect(authorName("<b>Jane Roe</b><script>x</script>")).toBe("Jane Roe");
  });
  it("refuses what is not a plain name", () => {
    for (const bad of [
      "",
      "Unknown author",
      "anonymous",
      "Own work",
      "The original uploader was King4057 at English Wikipedia.",
      "a Rockwell Automation Employee",
      "Raysonho @ Open Grid Scheduler / Scalable Grid Engine",
      "jane@example.com",
      "https://example.com/me",
      "www.example.com",
      "&lt;b&gt;Jane&lt;/b&gt;",
      "None",
      "x".repeat(61),
    ])
      expect(authorName(bad), bad).toBeNull();
  });
});

describe("stripFlickrPlace", () => {
  it("drops the Flickr importer's ' from <place>' only for a Flickr file", () => {
    expect(stripFlickrPlace("Peter Bond from Philadelphia, USA", true)).toBe("Peter Bond");
    expect(stripFlickrPlace("Parker Hannifin from Global, United States", true)).toBe("Parker Hannifin");
    expect(stripFlickrPlace("Peter Bond from Philadelphia, USA", false)).toBe("Peter Bond from Philadelphia, USA");
    expect(stripFlickrPlace("Coolcaesar", true)).toBe("Coolcaesar");
  });
});

describe("takenYear", () => {
  const now = new Date("2026-11-18T00:00:00Z");
  it("reads the clean forms of DateTimeOriginal", () => {
    expect(takenYear("2022:05:01 10:00:00", now)).toBe(2022);
    expect(takenYear("2022-05-01", now)).toBe(2022);
    expect(takenYear("2009-08", now)).toBe(2009);
    expect(takenYear("Taken on 8 June 2016, 11:08:51", now)).toBe(2016);
    expect(takenYear("28 October 2013", now)).toBe(2013);
    expect(takenYear("14 July 2021, 09:17:09 (according to Exif data)", now)).toBe(2021);
  });
  it("is null for an upload date, free text, a future year or nothing", () => {
    expect(takenYear("1 May 2008 (original upload date)", now)).toBeNull();
    expect(takenYear("circa 1990", now)).toBeNull();
    expect(takenYear("1990s", now)).toBeNull();
    expect(takenYear("2027-01-01", now)).toBeNull();
    expect(takenYear("", now)).toBeNull();
    expect(takenYear("8 Juni 2016", now)).toBeNull();
  });
  it("captionYears finds the years a caption names", () => {
    expect(captionYears("Intel's headquarters at 2200 Mission College Boulevard, in 2023.")).toEqual([2023]);
    expect(captionYears("Capital One 位于弗吉尼亚州的总部大楼，摄于2018年。")).toEqual([2018]);
    expect(captionYears("A Fab 5 building.")).toEqual([]);
  });
});

describe("licences", () => {
  it("allows public domain, CC0, CC BY and CC BY-SA in any version or port, and nothing else", () => {
    for (const ok of ["Public domain", "CC0", "CC0 1.0", "CC BY 2.0", "CC BY 4.0", "CC BY-SA 3.0", "CC BY-SA 3.0 de", "CC BY-SA 2.0 fr"]) expect(licenceAllowed(ok), ok).toBe(true);
    for (const bad of ["CC BY-NC 4.0", "CC BY-ND 2.0", "CC BY-NC-SA 3.0", "GFDL", "Attribution", "Copyrighted free use", "CC BY", "CC BY-SA 4.0 or GFDL", ""])
      expect(licenceAllowed(bad), bad).toBe(false);
  });
  it("rewrites a creativecommons.org licence URL to https and refuses any other host", () => {
    expect(licenceUrl("http://creativecommons.org/licenses/by-sa/3.0/")).toBe("https://creativecommons.org/licenses/by-sa/3.0/");
    expect(licenceUrl("https://creativecommons.org/publicdomain/zero/1.0/deed.en")).toBe("https://creativecommons.org/publicdomain/zero/1.0/deed.en");
    expect(licenceUrl("https://example.com/licenses/by/4.0")).toBeNull();
    expect(licenceUrl("http://example.com/creativecommons.org/")).toBeNull();
    expect(licenceUrl(null)).toBeNull();
  });
  it("accepts only an https Commons file page", () => {
    expect(commonsPage("https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg")).toBe("https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg");
    expect(commonsPage("http://commons.wikimedia.org/wiki/File:Intel_HQ.jpg")).toBeNull();
    expect(commonsPage("https://commons.wikimedia.org/wiki/User:X")).toBeNull();
  });
});

describe("headquartersNamed", () => {
  it("passes a caption without 'headquarters', and one that names a Wikidata headquarters place", () => {
    expect(headquartersNamed("A Target store in Ventura, California.", [])).toBe(true);
    expect(headquartersNamed("Intel's headquarters in Santa Clara, California.", ["Santa Clara", "Santa Clara County"])).toBe(true);
    expect(headquartersNamed("Lennar's headquarters in Miami-Dade County.", ["Miami, Florida"])).toBe(true);
  });
  it("fails one that names another place, or when Wikidata has none", () => {
    expect(headquartersNamed("Broadcom's headquarters in San Jose, California.", ["Palo Alto", "Santa Clara County"])).toBe(false);
    expect(headquartersNamed("Sysco's headquarters in Houston, Texas.", [])).toBe(false);
  });
});

describe("exifMismatch", () => {
  it("passes EXIF that names no one or names the credited author", () => {
    expect(exifMismatch("", ["X"])).toBeNull();
    expect(exifMismatch("unknown", ["X"])).toBeNull();
    expect(exifMismatch("Picasa 3.0", ["X"])).toBeNull();
    expect(exifMismatch("COPYRIGHT, 2007", ["X"])).toBeNull();
    expect(exifMismatch("Copyright", ["X"])).toBeNull();
    expect(exifMismatch("CC BY 2.0", ["X"])).toBeNull();
    expect(exifMismatch("©2019 Kenneth C. Zirkel", ["Kenneth C. Zirkel"])).toBeNull();
    expect(exifMismatch("Copyright 2021, G. Edward Johnson. CC-BY", ["G. Edward Johnson"])).toBeNull();
    expect(exifMismatch("Photographer: Dwight Burdette", ["Dwight Burdette"])).toBeNull();
    expect(exifMismatch("Jeroen Stroes", ["Jeroen Stroes Aviation Photography"])).toBeNull();
    expect(exifMismatch("Jason Zhang (Hian)", ["Jason Zhang"])).toBeNull();
    expect(exifMismatch("Ajay Suresh", ["ajay_suresh"])).toBeNull();
  });
  it("names someone else when the EXIF does (TGT: '© 2008 Paul Turang' on a file credited to Skinner2398)", () => {
    expect(exifMismatch("© 2008 Paul Turang", ["Skinner2398"])).toBe("Paul Turang");
    expect(exifMismatch("Paul Turang", ["Skinner2398", "Skinner2398"])).toBe("Paul Turang");
  });
});

describe("requestNotCovered", () => {
  it("is empty when the credit shows the requested attribution in full", () => {
    expect(requestNotCovered("Coolcaesar", "Coolcaesar", "CC BY-SA 3.0")).toEqual([]);
    expect(requestNotCovered("© Raimond Spekking / CC BY-SA 4.0 (via Wikimedia Commons)", "© Raimond Spekking", "CC BY-SA 4.0")).toEqual([]);
    expect(requestNotCovered("Gary Halvorson, Oregon State Archives / Cc-by-4.0", "Gary Halvorson, Oregon State Archives", "CC BY 4.0")).toEqual([]);
    expect(requestNotCovered("Coolcaesar at the English Wikipedia", "Coolcaesar at the English Wikipedia", "CC BY-SA 3.0")).toEqual([]);
  });
  it("lists what the credit would leave out: a ©, a web address, another name", () => {
    expect(requestNotCovered("© Raimond Spekking / CC BY-SA 4.0 (via Wikimedia Commons)", "Raimond Spekking", "CC BY-SA 4.0")).toEqual(["©"]);
    expect(requestNotCovered("Arne Müseler / www.arne-mueseler.com", "Arne Müseler", "CC BY-SA 3.0 de")).toEqual(["www", "mueseler", "com"]);
    expect(requestNotCovered("Photo by Jane Roe for Acme", "Jane Roe", "CC BY 4.0")).toEqual(["photo", "for", "acme"]);
  });
});

describe("permissionTerms", () => {
  it("drops Commons' licence box, the {{self}} line, a VRT notice and the licence restated, keeping a box's Attribution", () => {
    expect(
      permissionTerms(
        "This file is licensed under the Creative Commons Attribution 4.0 International license. Attribution: Gary Halvorson, Oregon State Archives / Cc-by-4.0 You are free: to share",
      ),
    ).toEqual({ conditions: "", attribution: "Gary Halvorson, Oregon State Archives / Cc-by-4.0" });
    expect(permissionTerms("Coolcaesar at the English-language Wikipedia, the copyright holder of this work, hereby publishes it under the following license:").conditions).toBe("");
    expect(
      permissionTerms(
        "This work is free and may be used by anyone for any purpose. If you wish to use this content, you do not need to request permission. This correspondence has been reviewed by a Volunteer Response Team (VRT) member.",
      ).conditions,
    ).toBe("");
    expect(permissionTerms("Public domain", "Public domain").conditions).toBe("");
  });
  it("keeps anything else, including a licence name that differs from the file's licence", () => {
    expect(permissionTerms("Please Ask If you Want It").conditions).toBe("Please Ask If you Want It");
    expect(permissionTerms("CC-By-SA-3.0", "CC BY 3.0").conditions).toBe("CC-By-SA-3.0");
    expect(permissionTerms("").conditions).toBe("");
  });
});

describe("originalSource", () => {
  it("takes the first link that leaves Wikimedia, with its title", () => {
    expect(originalSource('<a rel="nofollow" class="external text" href="https://www.flickr.com/photos/pvsbond/1580244122/">IMG_3987</a>')).toEqual({
      url: "https://www.flickr.com/photos/pvsbond/1580244122/",
      title: "IMG_3987",
      flickr: true,
    });
    expect(originalSource('<a href="https://www.flickr.com/photos/x/1/">https://www.flickr.com/photos/x/1/</a>')?.title).toBeNull();
    expect(originalSource('<span>Own work</span> <a href="//commons.wikimedia.org/wiki/File:X.jpg">x</a>')).toBeNull();
    expect(originalSource('Transferred from <a href="https://en.wikipedia.org">en.wikipedia</a>')).toBeNull();
  });
});
